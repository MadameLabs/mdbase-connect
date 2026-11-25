import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { MAX_CANDIDATES, gatherCandidates, searchTerms } from "./candidates.js";

type Row = { path: string; types?: string[]; revision?: string; values?: Record<string, unknown> };
/** results by term: the term is found in the `where` literal. */
function fake(byTerm: Record<string, Row[]>) {
  const operation = vi.fn(async (_s: string, _c: string, _op: string, input: { where: string }) => {
    const term = Object.keys(byTerm).find((t) => input.where.includes(`contains("${t}")`));
    const results = (term ? byTerm[term] : []).map((r) => ({ types: [], revision: "r", values: {}, ...r }));
    return { results, meta: {} };
  });
  return { gateway: { operation } as unknown as ConnectGateway, operation };
}
const wheres = (op: ReturnType<typeof fake>["operation"]) => op.mock.calls.map((c) => (c[3] as { where: string }).where);

describe("searchTerms", () => {
  it("separa termos longos e curtos, minúsculas, sem pontuação nem repetição", () => {
    expect(searchTerms("Fatura do cartão Max, PF e PJ")).toEqual({
      long: ["fatura", "cartão", "max"],
      short: ["do", "pf", "e", "pj"]
    });
    expect(searchTerms("Max max MAX!")).toEqual({ long: ["max"], short: [] });
  });
});

describe("gatherCandidates", () => {
  it("nota sem title nem description que bate no corpo continua candidata", async () => {
    const { gateway } = fake({ cartão: [{ path: "a.md", values: {} }] });
    const r = await gatherCandidates(gateway, "s", "c", "cartão", {});
    expect(r.candidates).toHaveLength(1);
    expect(r.candidates[0]).toMatchObject({ path: "a.md", title: null, description: null, hits: { body: 1, title: 0 }, preScore: 1 });
  });

  it("termos curtos geram consultas sem corpo e devolvem candidatas", async () => {
    const { gateway, operation } = fake({ pf: [{ path: "x-pf.md", values: { title: "Regras PF" } }], pj: [{ path: "y.md", values: { description: "sobre PJ" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "PF PJ", {});
    expect(operation).toHaveBeenCalledTimes(2);
    for (const w of wheres(operation)) {
      expect(w).not.toContain("file.body");
      expect(w).toContain("has(raw.title)");
      expect(w).toContain("has(raw.description)");
      expect(w).toContain("file.path.lower()");
    }
    expect(r.candidates.map((c) => c.path).sort()).toEqual(["x-pf.md", "y.md"]);
    expect(operation.mock.calls[0][3]).toMatchObject({ output: "metadata", select: ["title", "description"], limit: 1000 });
  });

  it("termo longo testa caminho, title, description e corpo", async () => {
    const { gateway, operation } = fake({});
    await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(wheres(operation)[0]).toContain('file.body.lower().contains("fatura")');
  });

  it("acerto no título pesa mais que no corpo", async () => {
    const { gateway } = fake({ fatura: [{ path: "b.md", values: {} }, { path: "a.md", values: { title: "Fatura de maio" } }] });
    const r = await gatherCandidates(gateway, "s", "c", "fatura", {});
    expect(r.candidates.map((c) => c.path)).toEqual(["a.md", "b.md"]);
    expect(r.candidates[0].preScore).toBe(3);
    expect(r.candidates[1].preScore).toBe(1);
  });

  it("soma o maior peso por termo distinto e desempata por número de termos", async () => {
    const { gateway } = fake({
      fatura: [{ path: "um.md", values: { title: "fatura" } }, { path: "dois.md", values: {} }],
      cartão: [{ path: "dois.md", values: {} }]
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
    await gatherCandidates(gateway, "s", "c", "x", { folder: 'a"b\\c' });
    expect(wheres(operation)[0]).toContain('startsWith("a\\"b\\\\c/")');
  });

  it("erro do gateway se propaga", async () => {
    const gateway = { operation: vi.fn(async () => { throw new Error("insufficient_collection_access"); }) } as unknown as ConnectGateway;
    await expect(gatherCandidates(gateway, "s", "c", "nota", {})).rejects.toThrow("insufficient_collection_access");
  });
});
