import { describe, expect, it, vi } from "vitest";
import { runtimeConfigFromEnv } from "../config.js";
import { JevError, createJevClient, type JevRankInput } from "./jev.js";
import { ANY_INDEX_INSTRUCTIONS, CANDIDATE_CRITERIA, CANDIDATE_INSTRUCTIONS, PICK_INDEX_INSTRUCTIONS } from "./jev-questions.js";

const KEY = "sk-secret-key-123";
const config = { apiKey: KEY, model: "jev-1.14.0" };

const input: JevRankInput = {
  query: "fatura do cartão",
  candidates: [
    { path: "a/um.md", title: "Um", description: "d1", excerpt: "trecho 1" },
    { path: "b/dois.md", title: null, description: null, excerpt: null }
  ],
  indexes: [
    { path: "fin/Índice Financeiro PF/PJ.md", title: "Índice Financeiro (PF/PJ)", description: "contas e cartões" },
    { path: "ops/indice.md", title: "Índice Financeiro (PF/PJ)", description: null }
  ]
};

function reply(status: number, body: unknown) {
  return new Response(typeof body === "string" ? body : JSON.stringify(body), { status });
}

const goodBody = {
  model: "jev-1.14.0",
  answers: {
    rel_c1: { type: "noul", noul: 0.9 },
    rel_c2: { type: "noul", noul: 0.1 },
    pick_index: { type: "choice", choice: "indice_financeiro_pf_pj_2", confidence: 0.7, probabilities: { indice_financeiro_pf_pj: 0.3, indice_financeiro_pf_pj_2: 0.7 } },
    any_index: { type: "noul", noul: 0.8 }
  }
};

function fakeFetch(response: () => Response | Promise<Response>) {
  return vi.fn(async (_url: unknown, _init?: RequestInit) => response());
}

describe("createJevClient", () => {
  it("faz uma requisição só, com um Noul por candidata, o Choice e o Noul dos indexes", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    await createJevClient(config, f as unknown as typeof fetch).rank(input);
    expect(f).toHaveBeenCalledTimes(1);
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("https://api.typesafe.ai/v1/systemone");
    expect(init?.method).toBe("POST");
    const body = JSON.parse(String(init?.body));
    expect(body.model).toBe("jev-1.14.0");
    expect(Object.keys(body.questions)).toEqual(["rel_c1", "rel_c2", "pick_index", "any_index"]);
    expect(body.questions.rel_c1).toEqual({
      type: "noul",
      instructions: CANDIDATE_INSTRUCTIONS("c1"),
      criteria: CANDIDATE_CRITERIA
    });
    expect(body.questions.rel_c1.instructions).toContain("`candidates.c1`");
    expect(body.questions.rel_c1.instructions).toContain("`query`");
    expect(body.state.query).toBe("fatura do cartão");
    expect(body.state.candidates).toEqual({
      c1: { path: "a/um.md", title: "Um", description: "d1", excerpt: "trecho 1" },
      c2: { path: "b/dois.md", title: null, description: null, excerpt: null }
    });
    expect(body.questions.pick_index.type).toBe("choice");
    expect(body.questions.pick_index.instructions).toBe(PICK_INDEX_INSTRUCTIONS);
    expect(body.questions.any_index.instructions).toBe(ANY_INDEX_INSTRUCTIONS);
  });

  it("nomes de opção descritivos, únicos e sem acento; o estado leva os indexes", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    await createJevClient(config, f as unknown as typeof fetch).rank(input);
    const body = JSON.parse(String(f.mock.calls[0]![1]?.body));
    expect(Object.keys(body.questions.pick_index.criteria)).toEqual(["indice_financeiro_pf_pj", "indice_financeiro_pf_pj_2"]);
    expect(body.questions.pick_index.criteria.indice_financeiro_pf_pj).toBe("contas e cartões");
    expect(body.questions.pick_index.criteria.indice_financeiro_pf_pj_2).toBe("Índice Financeiro (PF/PJ)");
    expect(body.state.indexes.indice_financeiro_pf_pj).toEqual({ path: "fin/Índice Financeiro PF/PJ.md", title: "Índice Financeiro (PF/PJ)", description: "contas e cartões" });
    expect(Object.keys(body.state.indexes)).toHaveLength(2);
  });

  it("o texto das candidatas fica no estado, nunca nas instruções", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    await createJevClient(config, f as unknown as typeof fetch).rank(input);
    const q = JSON.parse(String(f.mock.calls[0]![1]?.body)).questions;
    const text = JSON.stringify([q.rel_c1, q.rel_c2, q.any_index, q.pick_index.instructions]);
    for (const s of ["trecho 1", "a/um.md", "fatura do cartão"]) expect(text).not.toContain(s);
  });

  it("sem indexes omite indexes, pick_index e any_index", async () => {
    const f = fakeFetch(() =>
      reply(200, { answers: { rel_c1: { type: "noul", noul: 0.5 }, rel_c2: { type: "noul", noul: 0.4 } } })
    );
    const out = await createJevClient(config, f as unknown as typeof fetch).rank({ ...input, indexes: [] });
    const body = JSON.parse(String(f.mock.calls[0]![1]?.body));
    expect(Object.keys(body.questions)).toEqual(["rel_c1", "rel_c2"]);
    expect(body.state).not.toHaveProperty("indexes");
    expect(out.index).toBeNull();
    expect(out.anyIndexFits).toBeNull();
    expect(out.relevance.get("a/um.md")).toBe(0.5);
  });

  it("resposta válida vira relevance por caminho e o index escolhido", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    const out = await createJevClient(config, f as unknown as typeof fetch).rank(input);
    expect(out.relevance).toEqual(new Map([["a/um.md", 0.9], ["b/dois.md", 0.1]]));
    expect(out.index).toEqual({ path: "ops/indice.md", probability: 0.7 });
    expect(out.anyIndexFits).toBe(0.8);
  });

  it("a chave vai só no cabeçalho Authorization", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    await createJevClient(config, f as unknown as typeof fetch).rank(input);
    const [url, init] = f.mock.calls[0]!;
    const headers = new Headers(init?.headers);
    expect(headers.get("authorization")).toBe(`Bearer ${KEY}`);
    expect(headers.get("content-type")).toBe("application/json");
    expect(String(url)).not.toContain(KEY);
    expect(String(init?.body)).not.toContain(KEY);
  });

  it("repassa o signal", async () => {
    const f = fakeFetch(() => reply(200, goodBody));
    const ac = new AbortController();
    await createJevClient(config, f as unknown as typeof fetch).rank(input, ac.signal);
    expect(f.mock.calls[0]![1]?.signal).toBe(ac.signal);
  });

  async function failure(response: () => Response | Promise<Response>): Promise<JevError> {
    const f = fakeFetch(response);
    const err = await createJevClient(config, f as unknown as typeof fetch).rank(input).catch((e: unknown) => e);
    expect(f).toHaveBeenCalledTimes(1);
    expect(err).toBeInstanceOf(JevError);
    expect(JSON.stringify([(err as Error).message, (err as Error).stack])).not.toContain(KEY);
    return err as JevError;
  }

  it("429 vira jev_rate_limited", async () => {
    expect((await failure(() => reply(429, "slow"))).code).toBe("jev_rate_limited");
  });
  it("5xx, 529 e falha de rede viram jev_unavailable", async () => {
    expect((await failure(() => reply(500, ""))).code).toBe("jev_unavailable");
    expect((await failure(() => reply(529, ""))).code).toBe("jev_unavailable");
    expect((await failure(() => { throw new TypeError(`fetch failed ${KEY}`); })).code).toBe("jev_unavailable");
    const abort = await failure(() => { throw new DOMException("aborted", "AbortError"); });
    expect(abort.code).toBe("jev_unavailable");
  });
  it("401, 400 e 422 viram jev_unavailable com o status na mensagem", async () => {
    for (const status of [401, 400, 422]) {
      const e = await failure(() => reply(status, { detail: "x" }));
      expect(e.code).toBe("jev_unavailable");
      expect(e.message).toContain(String(status));
    }
  });
  it("corpo fora do formato vira jev_invalid_response", async () => {
    const bad: unknown[] = [
      "not json",
      {},
      { answers: null },
      { answers: { ...goodBody.answers, rel_c2: undefined } },
      { answers: { ...goodBody.answers, rel_c1: { type: "noul", noul: "0.9" } } },
      { answers: { ...goodBody.answers, any_index: { type: "choice" } } },
      { answers: { ...goodBody.answers, pick_index: { type: "choice", choice: "x", probabilities: { x: 1 } } } },
      { answers: { ...goodBody.answers, pick_index: { type: "choice", choice: "indice_financeiro_pf_pj", probabilities: { indice_financeiro_pf_pj: "a" } } } }
    ];
    for (const body of bad) expect((await failure(() => reply(200, body as string))).code).toBe("jev_invalid_response");
  });
});

describe("configuração do Jev", () => {
  const base = { MDBASE_MCP_MASTER_KEY: "k".repeat(40) };
  it("sem chave, jev é null", () => {
    expect(runtimeConfigFromEnv(base).jev).toBeNull();
    expect(runtimeConfigFromEnv({ ...base, MDBASE_MCP_TYPESAFE_API_KEY: "  " }).jev).toBeNull();
  });
  it("com chave, usa o modelo padrão ou o configurado", () => {
    expect(runtimeConfigFromEnv({ ...base, MDBASE_MCP_TYPESAFE_API_KEY: "k1" }).jev).toEqual({ apiKey: "k1", model: "jev-1.14.0" });
    expect(runtimeConfigFromEnv({ ...base, MDBASE_MCP_TYPESAFE_API_KEY: "k1", MDBASE_MCP_JEV_MODEL: "jev-latest" }).jev).toEqual({ apiKey: "k1", model: "jev-latest" });
  });
});
