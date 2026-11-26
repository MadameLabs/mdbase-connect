import { describe, expect, it, vi } from "vitest";
import { runtimeConfigFromEnv } from "../config.js";
import { EmbeddingError, createEmbeddingClient } from "./embeddings.js";

const KEY = "emb-secret-key-456";
const config = { apiKey: KEY, model: "emb-test-1", endpoint: "https://openrouter.ai/api/v1/embeddings", dimensions: 4 };

// Text "t<g>" comes back as [g + 1, 1, 0, 0]: its position in the whole call is encoded in the vector.
function vectorFor(text: string): number[] {
  return [Number(text.slice(1)) + 1, 1, 0, 0];
}

// Answers with the items in reverse order, so the client has to reorder them by `index`.
function embeddingReply(init?: RequestInit) {
  const body = JSON.parse(String(init?.body));
  const data = body.input.map((text: string, index: number) => ({ index, embedding: vectorFor(text) })).reverse();
  return new Response(JSON.stringify({ data, usage: { prompt_tokens: 1 } }), { status: 200 });
}

function reply(status: number, body: unknown) {
  return new Response(typeof body === "string" ? body : JSON.stringify(body), { status });
}

function fakeFetch(response: () => Response | Promise<Response>) {
  return vi.fn(async (_url: unknown, _init?: RequestInit) => response());
}

async function failureOf(f: unknown): Promise<EmbeddingError> {
  const err = await createEmbeddingClient(config, f as typeof fetch).embed(["t0"]).then(
    () => null,
    (e: unknown) => e
  );
  expect(err).toBeInstanceOf(EmbeddingError);
  return err as EmbeddingError;
}

describe("createEmbeddingClient", () => {
  it("divide em lotes de 64 e mantém a ordem pelo index", async () => {
    const f = vi.fn(async (_url: unknown, init?: RequestInit) => embeddingReply(init));
    const texts = Array.from({ length: 150 }, (_, g) => `t${g}`);
    const out = await createEmbeddingClient(config, f as unknown as typeof fetch).embed(texts);
    expect(f.mock.calls.map(([, init]) => JSON.parse(String(init?.body)).input.length)).toEqual([64, 64, 22]);
    expect(out).toHaveLength(150);
    out.forEach((v, g) => expect(v[0]).toBeCloseTo((g + 1) / Math.hypot(g + 1, 1), 6));
  });

  it("vetores voltam com norma 1, como Float32Array", async () => {
    const f = vi.fn(async (_url: unknown, init?: RequestInit) => embeddingReply(init));
    const out = await createEmbeddingClient(config, f as unknown as typeof fetch).embed(["t0", "t7"]);
    expect(out[0]).toBeInstanceOf(Float32Array);
    for (const v of out) expect(Math.hypot(...Array.from(v))).toBeCloseTo(1, 6);
  });

  it("manda modelo, textos e dimensões no corpo e a chave só no Authorization", async () => {
    const f = vi.fn(async (_url: unknown, init?: RequestInit) => embeddingReply(init));
    await createEmbeddingClient(config, f as unknown as typeof fetch).embed(["t0", "t1"]);
    expect(f).toHaveBeenCalledTimes(1);
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("https://openrouter.ai/api/v1/embeddings");
    expect(init?.method).toBe("POST");
    expect(init?.headers).toEqual({ "content-type": "application/json", authorization: `Bearer ${KEY}` });
    expect(JSON.parse(String(init?.body))).toEqual({ model: "emb-test-1", input: ["t0", "t1"], dimensions: 4 });
    expect(String(init?.body)).not.toContain(KEY);
  });

  it("repassa o signal", async () => {
    const ac = new AbortController();
    const f = vi.fn(async (_url: unknown, init?: RequestInit) => embeddingReply(init));
    await createEmbeddingClient(config, f as unknown as typeof fetch).embed(["t0"], ac.signal);
    expect(f.mock.calls[0]![1]?.signal).toBe(ac.signal);
  });

  it("lista vazia devolve vazio sem chamar a rede", async () => {
    const f = fakeFetch(() => reply(200, {}));
    expect(await createEmbeddingClient(config, f as unknown as typeof fetch).embed([])).toEqual([]);
    expect(f).not.toHaveBeenCalled();
  });

  it("429 vira embedding_rate_limited", async () => {
    const err = await failureOf(fakeFetch(() => reply(429, {})));
    expect(err.code).toBe("embedding_rate_limited");
  });

  it("HTTP 500 vira embedding_unavailable", async () => {
    const err = await failureOf(fakeFetch(() => reply(500, "erro interno")));
    expect(err.code).toBe("embedding_unavailable");
    expect(err.message).toContain("500");
  });

  it("mensagem de erro não ecoa o erro de transporte", async () => {
    const err = await failureOf(fakeFetch(() => Promise.reject(new Error(`socket hang up ${KEY}`))));
    expect(err.code).toBe("embedding_unavailable");
    expect(err.message).not.toContain("socket");
    expect(err.message).not.toContain(KEY);
  });

  it("JSON sem data vira embedding_invalid_response", async () => {
    const err = await failureOf(fakeFetch(() => reply(200, { usage: {} })));
    expect(err.code).toBe("embedding_invalid_response");
  });

  it("corpo que não é JSON vira embedding_invalid_response", async () => {
    const err = await failureOf(fakeFetch(() => reply(200, "não é json")));
    expect(err.code).toBe("embedding_invalid_response");
  });

  it("vetor com dimensão diferente de dimensions vira embedding_invalid_response", async () => {
    const err = await failureOf(fakeFetch(() => reply(200, { data: [{ index: 0, embedding: [1, 2, 3] }] })));
    expect(err.code).toBe("embedding_invalid_response");
  });

  it("index repetido ou ausente vira embedding_invalid_response", async () => {
    const repeated = await failureOf(
      fakeFetch(() => reply(200, { data: [{ index: 0, embedding: [1, 0, 0, 0] }, { index: 0, embedding: [0, 1, 0, 0] }] }))
    );
    expect(repeated.code).toBe("embedding_invalid_response");
    const missing = await failureOf(fakeFetch(() => reply(200, { data: [] })));
    expect(missing.code).toBe("embedding_invalid_response");
  });
});

describe("configuração dos embeddings", () => {
  const base = { MDBASE_MCP_MASTER_KEY: "k".repeat(40) };
  const collection = {
    collectionId: "11111111-1111-4111-8111-111111111111",
    connectionSetId: "22222222-2222-4222-8222-222222222222",
    connectionId: "33333333-3333-4333-8333-333333333333"
  };
  const collectionsEnv = `${collection.collectionId}:${collection.connectionSetId}:${collection.connectionId}`;

  it("sem modelo, embedding é null", () => {
    expect(runtimeConfigFromEnv(base).embedding).toBeNull();
    expect(runtimeConfigFromEnv({ ...base, MDBASE_MCP_TYPESAFE_API_KEY: "k1" }).embedding).toBeNull();
  });

  it("lê modelo, endpoint padrão, dimensões, coleções e intervalo", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: " emb-test-1 ", MDBASE_MCP_EMBEDDING_API_KEY: "k2", MDBASE_MCP_EMBEDDING_COLLECTIONS: ` ${collectionsEnv} ,` };
    expect(runtimeConfigFromEnv(env).embedding).toEqual({
      apiKey: "k2",
      model: "emb-test-1",
      endpoint: "https://openrouter.ai/api/v1/embeddings",
      dimensions: 1024,
      collections: [collection],
      intervalMs: 300_000
    });
  });

  it("a chave cai na do Jev quando a de embedding não existe", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1", MDBASE_MCP_TYPESAFE_API_KEY: "k1" };
    expect(runtimeConfigFromEnv(env).embedding?.apiKey).toBe("k1");
    expect(runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_API_KEY: "k2" }).embedding?.apiKey).toBe("k2");
  });

  it("modelo sem chave nenhuma recusa a configuração", () => {
    expect(() => runtimeConfigFromEnv({ ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1" })).toThrow(/MDBASE_MCP_EMBEDDING_API_KEY/);
  });

  it("coleções malformadas recusam", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1", MDBASE_MCP_TYPESAFE_API_KEY: "k1" };
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_COLLECTIONS: "x:y:z" })).toThrow(/MDBASE_MCP_EMBEDDING_COLLECTIONS/);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_COLLECTIONS: `${collection.collectionId}:${collection.connectionSetId}` })).toThrow(
      /MDBASE_MCP_EMBEDDING_COLLECTIONS/
    );
  });

  it("recusa endpoint sem HTTPS ou com credencial", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1", MDBASE_MCP_EMBEDDING_API_KEY: "k2" };
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_ENDPOINT: "http://openrouter.ai/api/v1/embeddings" })).toThrow(/MDBASE_MCP_EMBEDDING_ENDPOINT/);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_ENDPOINT: "https://u:p@openrouter.ai/api/v1/embeddings" })).toThrow(/MDBASE_MCP_EMBEDDING_ENDPOINT/);
  });

  it("dimensões são inteiro de 64 a 4096", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1", MDBASE_MCP_EMBEDDING_API_KEY: "k2" };
    expect(runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_DIMENSIONS: "64" }).embedding?.dimensions).toBe(64);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_DIMENSIONS: "63" })).toThrow(/MDBASE_MCP_EMBEDDING_DIMENSIONS/);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_DIMENSIONS: "4097" })).toThrow(/MDBASE_MCP_EMBEDDING_DIMENSIONS/);
  });

  it("intervalo é de 30 a 3600 segundos", () => {
    const env = { ...base, MDBASE_MCP_EMBEDDING_MODEL: "emb-test-1", MDBASE_MCP_EMBEDDING_API_KEY: "k2" };
    expect(runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS: "30" }).embedding?.intervalMs).toBe(30_000);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS: "29" })).toThrow(/MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS/);
    expect(() => runtimeConfigFromEnv({ ...env, MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS: "3601" })).toThrow(/MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS/);
  });
});
