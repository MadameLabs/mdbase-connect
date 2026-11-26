import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { BODY_TERMS, MAX_CANDIDATES, bodyExpression, exactTermCandidates, folderRange, gatherCandidates, metaExpression, searchTerms } from "./candidates.js";

type Row = { path: string; types?: string[]; revision?: string; frontmatter?: Record<string, unknown> };
const BODY_RE = /^\(?file\.body\.lower\(\)\.contains\("([^"]*)"\)/;
const isBodyWhere = (where: string) => BODY_RE.test(where);
/** One body query per long term (`body[term]`, served `pageSize` rows a page with a
 * numeric cursor) and one title/description/path query (`meta`). */
function fake(lists: { body?: Record<string, Row[]>; meta?: Row[]; pageSize?: number }) {
  const operation = vi.fn(async (_s: string, _c: string, _op: string, input: { where: string; cursor?: string }) => {
    const m = BODY_RE.exec(input.where);
    const all = (m ? lists.body?.[m[1]] : lists.meta) ?? [];
    const size = m ? lists.pageSize ?? Infinity : Infinity;
    const from = input.cursor ? Number(input.cursor) : 0;
    const source = all.slice(from, from + size);
    const results = source.map((r) => ({ path: r.path, types: r.types ?? [], revision: r.revision ?? "r", frontmatter: r.frontmatter ?? {} }));
    const next = from + size < all.length ? String(from + size) : null;
    return { results, meta: next ? { has_more: true, cursor: next } : { has_more: false } };
  });
  return { gateway: { operation } as unknown as ConnectGateway, operation };
}
const wheres = (op: ReturnType<typeof fake>["operation"]) => op.mock.calls.map((c) => (c[3] as { where: string }).where);
const bodyWheres = (op: ReturnType<typeof fake>["operation"]) => wheres(op).filter(isBodyWhere);
const metaWhere = (op: ReturnType<typeof fake>["operation"]) => wheres(op).find((w) => !isBodyWhere(w));

describe("searchTerms", () => {
  it("separa termos longos e curtos, minúsculas, sem pontuação nem repetição", () => {
    expect(searchTerms("Fatura do cartão Max, PF e PJ")).toEqual({
      long: ["fatura", "cartão", "max"],
      short: ["pf", "pj"]
    });
    expect(searchTerms("Max max MAX!")).toEqual({ long: ["max"], short: [] });
  });
  it("descarta termos de uma letra e palavras vazias, mantém siglas de duas letras", () => {
    expect(searchTerms("a o de do da em no na os as um uma com por para que dos das ao aos nos nas x")).toEqual({ long: [], short: [] });
    expect(searchTerms("IA e PF")).toEqual({ long: [], short: ["ia", "pf"] });
  });
});

describe("searchTerms: teto", () => {
  it("mantém no máximo 8 termos, os mais longos, empate pela primeira aparição", () => {
    const paragraph = "o gestor de projetos pediu uma analise detalhada sobre custos operacionais mensais da empresa incluindo ia pf pj e fornecedores externos";
    const { long, short } = searchTerms(paragraph);
    expect(long.length + short.length).toBe(8);
    expect(long).toEqual(["projetos", "analise", "detalhada", "operacionais", "mensais", "incluindo", "fornecedores", "externos"]);
    expect(short).toEqual([]);
  });
  it("siglas contam no teto e perdem para termos mais longos", () => {
    const { long, short } = searchTerms("ia aaa bbb ccc ddd eee fff ggg hhh");
    expect(long).toEqual(["aaa", "bbb", "ccc", "ddd", "eee", "fff", "ggg", "hhh"]);
    expect(short).toEqual([]);
    expect(searchTerms("ia aaa").short).toEqual(["ia"]);
  });
  it("stopwords em inglês saem", () => {
    expect(searchTerms("the and of to in for on with an is are at by from or ia")).toEqual({ long: [], short: ["ia"] });
  });
});

describe("expressões", () => {
  it("bodyExpression é só o contains do corpo, para o índice", () => {
    expect(bodyExpression("cartão")).toBe('file.body.lower().contains("cartão")');
  });
  it("metaExpression testa título, descrição e caminho; sigla por fronteira de palavra", () => {
    const w = metaExpression(["fatura"], ["pf"]);
    expect(w).toContain('(has(raw.title) && raw.title.lower().contains("fatura"))');
    expect(w).toContain('(has(raw.description) && raw.description.lower().contains("fatura"))');
    expect(w).toContain('file.path.lower().contains("fatura")');
    expect(w).toContain('file.path.lower().matches("(^|[^a-z0-9à-ÿ])pf([^a-z0-9à-ÿ]|$)")');
    expect(w).not.toContain("file.body");
  });
  it("folderRange é uma faixa de caminho, sem barra dupla e com escape", () => {
    expect(folderRange("13-engenharia")).toBe('(file.path >= "13-engenharia/" && file.path < "13-engenharia0")');
    expect(folderRange("a/b/")).toBe('(file.path >= "a/b/" && file.path < "a/b0")');
    expect(folderRange('a"b')).toBe('(file.path >= "a\\"b/" && file.path < "a\\"b0")');
  });
});

describe("gatherCandidates", () => {
  it("uma consulta de corpo por termo longo e uma de título, descrição e caminho", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "fatura cartão PF", {});
    expect(operation).toHaveBeenCalledTimes(3);
    expect(bodyWheres(operation).sort()).toEqual(['file.body.lower().contains("cartão")', 'file.body.lower().contains("fatura")']);
    const meta = metaWhere(operation) as string;
    expect(meta).toContain('raw.title.lower().contains("fatura")');
    expect(meta).toContain('raw.title.lower().contains("cartão")');
    expect(meta).toContain('file.path.lower().matches("(^|[^a-z0-9à-ÿ])pf([^a-z0-9à-ÿ]|$)")');
    expect(meta).not.toContain("file.body");
    for (const c of operation.mock.calls) {
      const input = c[3] as Record<string, unknown>;
      expect(input.limit).toBe(300);
      // A consulta comum: a autoridade hospedada recusa `output: "metadata"` com condição no corpo.
      expect(input).not.toHaveProperty("output");
      expect(input).not.toHaveProperty("select");
      expect(input).not.toHaveProperty("types");
    }
  });

  it(`corpo só para os ${BODY_TERMS} termos longos mais longos`, async () => {
    // searchTerms keeps MAX_TERMS (8) terms, so "abc" (the shortest) is cut before BODY_TERMS applies.
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "abc abcd abcde abcdef abcdefg abcdefgh abcdefghi abcdefghij abcdefghijk", {});
    expect(bodyWheres(operation).map((w) => BODY_RE.exec(w)?.[1]).sort()).toEqual([
      "abcd", "abcde", "abcdef", "abcdefg", "abcdefgh", "abcdefghi", "abcdefghij", "abcdefghijk"
    ]);
    expect(metaWhere(operation)).toContain('raw.title.lower().contains("abcd")');
  });

  it("só siglas: uma consulta, sem corpo", async () => {
    const { gateway, operation } = fake({ meta: [{ path: "x-pf.md", frontmatter: { title: "Regras PF" } }, { path: "y.md", frontmatter: { description: "sobre PJ" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "PF PJ", {});
    expect(operation).toHaveBeenCalledTimes(1);
    expect(wheres(operation)[0]).not.toContain("file.body");
    expect(wheres(operation)[0]).not.toContain(".contains(");
    expect(r.candidates.map((c) => c.path).sort()).toEqual(["x-pf.md", "y.md"]);
  });

  it("pasta vira faixa de caminho em cada consulta do corpo", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "fatura cartão", { types: ["task"], folder: "13-engenharia" });
    expect(bodyWheres(operation).sort()).toEqual([
      '(file.body.lower().contains("cartão")) && (file.path >= "13-engenharia/" && file.path < "13-engenharia0")',
      '(file.body.lower().contains("fatura")) && (file.path >= "13-engenharia/" && file.path < "13-engenharia0")'
    ]);
    expect(metaWhere(operation)).toContain('file.path.startsWith("13-engenharia/")');
    for (const c of operation.mock.calls) expect((c[3] as unknown as { types: string[] }).types).toEqual(["task"]);
  });

  it("escapa aspas e barras do literal da pasta", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "xx", { folder: 'a"b\\c' });
    expect(metaWhere(operation)).toContain('startsWith("a\\"b\\\\c/")');
  });

  it("termo médio: o corpo segue o cursor até o fim", async () => {
    const rows = Array.from({ length: 5 }, (_, i) => ({ path: `s${i}.md` }));
    const { gateway, operation } = fake({ body: { sudo: rows }, pageSize: 2 });
    const r = await gatherCandidates(gateway, "s", "c", "sudo", {});
    const sudoCalls = operation.mock.calls.filter((c) => isBodyWhere((c[3] as { where: string }).where));
    expect(sudoCalls.map((c) => (c[3] as { cursor?: string }).cursor)).toEqual([undefined, "2", "4"]);
    for (const c of sudoCalls) expect((c[3] as { where: string }).where).toBe('file.body.lower().contains("sudo")');
    expect(r.total).toBe(5);
    expect(r.candidates.every((c) => c.hits.body === 1)).toBe(true);
  });

  it("termo comum (mais de 300 notas no corpo): descartado, sem passar de 300", async () => {
    const rows = Array.from({ length: 301 }, (_, i) => ({ path: `r${i}.md` }));
    const { gateway, operation } = fake({ body: { reuniao: rows, fatura: [{ path: "a.md" }] }, pageSize: 100 });
    const r = await gatherCandidates(gateway, "s", "c", "reuniao fatura", {});
    const reuniao = operation.mock.calls.filter((c) => (c[3] as { where: string }).where.includes('"reuniao")') && isBodyWhere((c[3] as { where: string }).where));
    expect(reuniao).toHaveLength(3);
    expect(r.candidates.map((c) => c.path)).toEqual(["a.md"]);
  });

  it("corpo conta por termo, só onde o termo não bateu fora dele", async () => {
    const a = { path: "a.md", frontmatter: { title: "Fatura" } };
    const { gateway } = fake({ body: { fatura: [a], cartão: [a] }, meta: [a] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura cartão", {});
    expect(r.total).toBe(1);
    expect(r.candidates[0]).toMatchObject({ preScore: 4, hits: { title: 1, description: 0, path: 0, body: 1 } });
  });

  it("termo no título e no corpo conta só o peso do título", async () => {
    const a = { path: "a.md", frontmatter: { title: "Fatura" } };
    const { gateway } = fake({ body: { fatura: [a] }, meta: [a] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(r.candidates[0].preScore).toBe(3);
    expect(r.candidates[0].hits).toEqual({ title: 1, description: 0, path: 0, body: 0 });
  });

  it("nota só do corpo vale 1 por termo e total conta notas distintas", async () => {
    const b = { path: "b.md", frontmatter: { title: "Fatura" } };
    const { gateway } = fake({ body: { fatura: [{ path: "a.md" }, b] }, meta: [b] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(r.total).toBe(2);
    expect(r.candidates.map((c) => [c.path, c.preScore])).toEqual([["b.md", 3], ["a.md", 1]]);
    expect(r.candidates[1].hits).toEqual({ title: 0, description: 0, path: 0, body: 1 });
  });

  it("nota sem title nem description que bate no corpo continua candidata", async () => {
    const { gateway } = fake({ body: { cartão: [{ path: "a.md", frontmatter: {} }] } });
    const r = await gatherCandidates(gateway, "s", "c", "cartão", {});
    expect(r.candidates).toHaveLength(1);
    expect(r.candidates[0]).toMatchObject({ path: "a.md", title: null, description: null, hits: { body: 1, title: 0 }, preScore: 1 });
  });

  it("soma o maior peso por termo distinto e desempata por número de termos", async () => {
    const um = { path: "um.md", frontmatter: { title: "fatura" } };
    const { gateway } = fake({ body: { fatura: [{ path: "dois.md" }], cartão: [{ path: "dois.md" }, { path: "tres.md" }] }, meta: [um] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura cartão", {});
    expect(r.candidates.map((c) => [c.path, c.preScore])).toEqual([["um.md", 3], ["dois.md", 2], ["tres.md", 1]]);
  });

  it("mais de 30 candidatas: devolve 30 e total real", async () => {
    const rows = Array.from({ length: 45 }, (_, i) => ({ path: `n${i}.md` }));
    const { gateway } = fake({ body: { nota: rows } });
    const r = await gatherCandidates(gateway, "s", "c", "nota", {});
    expect(r.candidates).toHaveLength(MAX_CANDIDATES);
    expect(r.total).toBe(45);
  });

  it("só palavras vazias: nenhuma consulta e resultado vazio", async () => {
    const { gateway, operation } = fake({});
    expect(await gatherCandidates(gateway, "s", "c", "de a o para", {})).toEqual({ candidates: [], total: 0 });
    expect(await gatherCandidates(gateway, "s", "c", "", {})).toEqual({ candidates: [], total: 0 });
    expect(operation).not.toHaveBeenCalled();
  });

  it("sigla casa por palavra: ia não acerta em 'media'", async () => {
    const { gateway } = fake({ meta: [
      { path: "midia.md", frontmatter: { title: "Mídia social" } },
      { path: "x/ia-geral.md", frontmatter: {} },
      { path: "y.md", frontmatter: { title: "Uso de IA no dia a dia" } }
    ] });
    const r = await gatherCandidates(gateway, "s", "c", "ia", {});
    const by = Object.fromEntries(r.candidates.map((c) => [c.path, c.hits]));
    expect(by["midia.md"]).toEqual({ title: 0, description: 0, path: 0, body: 0 });
    expect(by["x/ia-geral.md"].path).toBe(1);
    expect(by["y.md"].title).toBe(1);
  });

  it("types vêm do resultado, ou vazio", async () => {
    const { gateway } = fake({ body: { nota: [{ path: "a.md", types: ["task"] }, { path: "b.md" }] } });
    const r = await gatherCandidates(gateway, "s", "c", "nota", {});
    expect(r.candidates.map((c) => c.types).sort()).toEqual([[], ["task"]]);
  });

  it("erro do gateway se propaga", async () => {
    const gateway = { operation: vi.fn(async () => { throw new Error("insufficient_collection_access"); }) } as unknown as ConnectGateway;
    await expect(gatherCandidates(gateway, "s", "c", "nota", {})).rejects.toThrow("insufficient_collection_access");
  });
});

describe("exactTermCandidates", () => {
  it("termos exatos: corpo por termo e meta com todos", async () => {
    const { gateway, operation } = fake({
      body: { fatura: [{ path: "a.md", types: ["task"], revision: "r1" }], cartão: [{ path: "b.md" }] },
      meta: [{ path: "m.md", frontmatter: { title: "Fatura do cartão" } }]
    });
    const r = await exactTermCandidates(gateway, "s", "c", ["fatura", "cartão"], {}, { deadlineMs: 1500 });
    expect(bodyWheres(operation).sort()).toEqual(['file.body.lower().contains("cartão")', 'file.body.lower().contains("fatura")']);
    const meta = metaWhere(operation) as string;
    expect(meta).toContain('raw.title.lower().contains("fatura")');
    expect(meta).toContain('raw.title.lower().contains("cartão")');
    expect(meta).not.toContain("file.body");
    expect([...r.paths.keys()].sort()).toEqual(["a.md", "b.md", "m.md"]);
    expect(r.paths.get("a.md")).toEqual({ title: null, description: null, types: ["task"], revision: "r1" });
    expect(r.paths.get("m.md")).toEqual({ title: "Fatura do cartão", description: null, types: [], revision: "r" });
    expect(r.dropped).toEqual([]);
  });

  it("sigla de 2 letras só na meta", async () => {
    const { gateway, operation } = fake({ meta: [{ path: "x-pf.md", frontmatter: { title: "Regras PF" } }] });
    const r = await exactTermCandidates(gateway, "s", "c", ["pf"], {}, { deadlineMs: 1500 });
    expect(operation).toHaveBeenCalledTimes(1);
    expect(wheres(operation)[0]).not.toContain("file.body");
    expect(wheres(operation)[0]).toContain('file.path.lower().matches("(^|[^a-z0-9à-ÿ])pf([^a-z0-9à-ÿ]|$)")');
    expect([...r.paths.keys()]).toEqual(["x-pf.md"]);
  });

  it("termo comum acima de 300 descartado", async () => {
    const rows = Array.from({ length: 301 }, (_, i) => ({ path: `r${i}.md` }));
    const { gateway, operation } = fake({ body: { reuniao: rows, fatura: [{ path: "a.md" }] }, pageSize: 100 });
    const r = await exactTermCandidates(gateway, "s", "c", ["reuniao", "fatura"], {}, { deadlineMs: 1500 });
    const reuniao = bodyWheres(operation).filter((w) => w.includes('"reuniao"'));
    expect(reuniao).toHaveLength(3);
    expect([...r.paths.keys()]).toEqual(["a.md"]);
    expect(r.dropped).toEqual([]);
  });

  it("termo que estoura o prazo vai para dropped e os outros voltam", async () => {
    vi.useFakeTimers();
    try {
      const operation = vi.fn(async (_s: string, _c: string, _op: string, input: { where: string }) => {
        if (input.where === 'file.body.lower().contains("lento")') return new Promise<never>(() => {});
        if (input.where === 'file.body.lower().contains("rapido")') {
          return { results: [{ path: "r.md", types: [], revision: "r", frontmatter: {} }], meta: { has_more: false } };
        }
        return { results: [], meta: { has_more: false } };
      });
      const gateway = { operation } as unknown as ConnectGateway;
      const pending = exactTermCandidates(gateway, "s", "c", ["lento", "rapido"], {}, { deadlineMs: 1500 });
      await vi.advanceTimersByTimeAsync(1500);
      const r = await pending;
      expect(r.dropped).toEqual(["lento"]);
      expect([...r.paths.keys()]).toEqual(["r.md"]);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("reserva e prazo", () => {
  it("BODY_TERMS é 8 na reserva", () => {
    expect(BODY_TERMS).toBe(8);
  });
});
