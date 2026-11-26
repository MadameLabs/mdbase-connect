import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { MAX_CANDIDATES, gatherCandidates, searchTerms } from "./candidates.js";

type Row = { path: string; types?: string[]; revision?: string; frontmatter?: Record<string, unknown> };
/** results by term: the term is found in the `where` literal. `more` lists terms whose
 * full (body) query reports has_more; their restricted re-query gets `restricted[term]`. */
function fake(byTerm: Record<string, Row[]>, more: string[] = [], restricted: Record<string, Row[]> = {}) {
  const operation = vi.fn(async (_s: string, _c: string, _op: string, input: { where: string }) => {
    const term = Object.keys(byTerm).find((t) => input.where.includes(`contains("${t}")`) || input.where.includes(`])${t}([`));
    const isBody = input.where.includes("file.body");
    const source = term && more.includes(term) && !isBody ? restricted[term] ?? [] : term ? byTerm[term] : [];
    const results = source.map((r) => ({ path: r.path, types: r.types ?? [], revision: r.revision ?? "r", frontmatter: r.frontmatter ?? {} }));
    return { results, meta: { has_more: term !== undefined && more.includes(term) && isBody } };
  });
  return { gateway: { operation } as unknown as ConnectGateway, operation };
}
const wheres = (op: ReturnType<typeof fake>["operation"]) => op.mock.calls.map((c) => (c[3] as { where: string }).where);

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

describe("gatherCandidates", () => {
  it("nota sem title nem description que bate no corpo continua candidata", async () => {
    const { gateway } = fake({ cartão: [{ path: "a.md", frontmatter: {} }] });
    const r = await gatherCandidates(gateway, "s", "c", "cartão", {});
    expect(r.candidates).toHaveLength(1);
    expect(r.candidates[0]).toMatchObject({ path: "a.md", title: null, description: null, hits: { body: 1, title: 0 }, preScore: 1 });
  });

  it("termos curtos geram consultas sem corpo e devolvem candidatas", async () => {
    const { gateway, operation } = fake({ pf: [{ path: "x-pf.md", frontmatter: { title: "Regras PF" } }], pj: [{ path: "y.md", frontmatter: { description: "sobre PJ" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "PF PJ", {});
    expect(operation).toHaveBeenCalledTimes(2);
    for (const w of wheres(operation)) {
      expect(w).not.toContain("file.body");
      expect(w).toContain("has(raw.title)");
      expect(w).toContain("has(raw.description)");
      expect(w).toContain("file.path.lower().matches(");
      expect(w).not.toContain(".contains(");
    }
    expect(wheres(operation)[0]).toContain('"(^|[^a-z0-9à-ÿ])pf([^a-z0-9à-ÿ]|$)"');
    expect(r.candidates.map((c) => c.path).sort()).toEqual(["x-pf.md", "y.md"]);
    const input = operation.mock.calls[0][3] as Record<string, unknown>;
    expect(input.limit).toBe(300);
    // A consulta comum: a autoridade hospedada recusa `output: "metadata"` com condição
    // no corpo (query_budget_exceeded), mesmo com poucos resultados.
    expect(input).not.toHaveProperty("output");
    expect(input).not.toHaveProperty("select");
    expect(input).not.toHaveProperty("include_body");
    expect(input).not.toHaveProperty("types");
  });

  it("termo longo testa caminho, title, description e corpo", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(wheres(operation)[0]).toContain('file.body.lower().contains("fatura")');
  });

  it("acerto no título pesa mais que no corpo", async () => {
    const { gateway } = fake({ fatura: [{ path: "b.md", frontmatter: {} }, { path: "a.md", frontmatter: { title: "Fatura de maio" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(r.candidates.map((c) => c.path)).toEqual(["a.md", "b.md"]);
    expect(r.candidates[0].preScore).toBe(3);
    expect(r.candidates[1].preScore).toBe(1);
  });

  it("soma o maior peso por termo distinto e desempata por número de termos", async () => {
    const { gateway } = fake({
      fatura: [{ path: "um.md", frontmatter: { title: "fatura" } }, { path: "dois.md", frontmatter: {} }],
      cartão: [{ path: "dois.md", frontmatter: {} }]
    });
    const r = await gatherCandidates(gateway, "s", "c", "fatura cartão", {});
    expect(r.candidates.map((c) => [c.path, c.preScore])).toEqual([["um.md", 3], ["dois.md", 2]]);
  });

  it("mais de 30 candidatas: devolve 30 e total real", async () => {
    const rows = Array.from({ length: 45 }, (_, i) => ({ path: `n${i}.md` }));
    const { gateway } = fake({ nota: rows });
    const r = await gatherCandidates(gateway, "s", "c", "nota", {});
    expect(r.candidates).toHaveLength(MAX_CANDIDATES);
    expect(r.total).toBe(45);
  });

  it("filtros types e folder chegam à query", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "nota", { types: ["task"], folder: "13-engenharia" });
    const input = operation.mock.calls[0][3] as { where: string; types: string[] };
    expect(input.types).toEqual(["task"]);
    expect(input.where).toContain('file.path.startsWith("13-engenharia/")');
    await gatherCandidates(gateway, "s", "c", "nota", { folder: "a/b/" });
    expect(wheres(operation)[1]).toContain('startsWith("a/b/")');
  });

  it("escapa aspas e barras do literal", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "xx", { folder: 'a"b\\c' });
    expect(wheres(operation)[0]).toContain('startsWith("a\\"b\\\\c/")');
  });

  it("só palavras vazias: nenhuma consulta e resultado vazio", async () => {
    const { gateway, operation } = fake({});
    expect(await gatherCandidates(gateway, "s", "c", "de a o para", {})).toEqual({ candidates: [], total: 0 });
    expect(await gatherCandidates(gateway, "s", "c", "", {})).toEqual({ candidates: [], total: 0 });
    expect(operation).not.toHaveBeenCalled();
  });

  it("termo no título e no corpo conta só o peso do título", async () => {
    const { gateway } = fake({ fatura: [{ path: "a.md", frontmatter: { title: "Fatura" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(r.candidates[0].preScore).toBe(3);
    expect(r.candidates[0].hits).toEqual({ title: 1, description: 0, path: 0, body: 0 });
  });

  it("sigla casa por palavra: ia não acerta em 'media'", async () => {
    const { gateway } = fake({ ia: [
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

  it("termo comum (has_more): refaz sem corpo, usa esse resultado e não dá crédito de corpo", async () => {
    const { gateway, operation } = fake(
      { reuniao: [{ path: "corpo.md" }, { path: "t.md", frontmatter: { title: "Reunião" } }] },
      ["reuniao"],
      { reuniao: [{ path: "t.md", frontmatter: { title: "reuniao semanal" } }] }
    );
    const r = await gatherCandidates(gateway, "s", "c", "reuniao", {});
    expect(operation).toHaveBeenCalledTimes(2);
    const [first, second] = wheres(operation);
    expect(first).toContain("file.body");
    expect(second).not.toContain("file.body");
    expect((operation.mock.calls[1][3] as unknown as { limit: number }).limit).toBe(300);
    expect(r.candidates.map((c) => c.path)).toEqual(["t.md"]);
    expect(r.candidates[0].hits).toEqual({ title: 1, description: 0, path: 0, body: 0 });
  });

  it("cursor também marca termo comum", async () => {
    const operation = vi.fn(async (_s: string, _c: string, _o: string, input: { where: string }) =>
      ({ results: [], meta: input.where.includes("file.body") ? { has_more: false, cursor: "abc" } : {} }));
    await gatherCandidates({ operation } as unknown as ConnectGateway, "s", "c", "reuniao", {});
    expect(operation).toHaveBeenCalledTimes(2);
  });

  it("termo curto com has_more não é refeito", async () => {
    const operation = vi.fn(async () => ({ results: [], meta: { has_more: true } }));
    await gatherCandidates({ operation } as unknown as ConnectGateway, "s", "c", "ia", {});
    expect(operation).toHaveBeenCalledTimes(1);
  });

  it("termo curto sem acerto em título, descrição ou caminho não conta como corpo", async () => {
    const { gateway } = fake({ pf: [{ path: "a.md", frontmatter: {} }] });
    const r = await gatherCandidates(gateway, "s", "c", "pf", {});
    expect(r.candidates[0].hits).toEqual({ title: 0, description: 0, path: 0, body: 0 });
  });

  it("types vêm do resultado, ou vazio", async () => {
    const { gateway } = fake({ nota: [{ path: "a.md", types: ["task"] }, { path: "b.md" }] });
    const r = await gatherCandidates(gateway, "s", "c", "nota", {});
    expect(r.candidates.map((c) => c.types).sort()).toEqual([[], ["task"]]);
  });

  it("erro do gateway se propaga", async () => {
    const gateway = { operation: vi.fn(async () => { throw new Error("insufficient_collection_access"); }) } as unknown as ConnectGateway;
    await expect(gatherCandidates(gateway, "s", "c", "nota", {})).rejects.toThrow("insufficient_collection_access");
  });
});
