import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { INDEX_BOOST, IndexListCache, MAX_INDEX_OPTIONS } from "./indexes.js";

const CONN = "01911111-1111-7111-8111-111111111111";

function gatewayWith(queryImpl: () => Promise<unknown>, operations = ["read", "query"]) {
  const operation = vi.fn(async (_s: string, _c: string, op: string) => {
    if (op === "query") return queryImpl();
    throw new Error(`unexpected ${op}`);
  });
  const listConnections = vi.fn(async () => [
    { id: CONN, collection_id: "c", display_name: "x", operations, scope: {}, authority: "local" }
  ]);
  return { operation, listConnections };
}
const asGateway = (g: ReturnType<typeof gatewayWith>) => g as unknown as ConnectGateway;
const queryCalls = (g: ReturnType<typeof gatewayWith>) => (g.operation.mock.calls as unknown[][]).filter((c) => c[2] === "query");

describe("IndexListCache", () => {
  it("exporta as constantes", () => {
    expect(MAX_INDEX_OPTIONS).toBe(255);
    expect(INDEX_BOOST).toBe(2);
  });

  it("monta a lista com fallback de título e descrição null", async () => {
    const gateway = gatewayWith(async () => ({
      results: [
        { path: "b/Hub_B.md", revision: "r", frontmatter: { title: "Hub B", description: "sobre B" } },
        { path: "a/Hub_A.md", revision: "r", frontmatter: { description: 3 } },
        { path: "Raiz.md", revision: "r", frontmatter: { title: 7 } }
      ]
    }));
    const cache = new IndexListCache(asGateway(gateway));
    expect(await cache.list("set-1", CONN)).toEqual([
      { path: "Raiz.md", title: "Raiz", description: null },
      { path: "a/Hub_A.md", title: "Hub_A", description: null },
      { path: "b/Hub_B.md", title: "Hub B", description: "sobre B" }
    ]);
    expect(queryCalls(gateway)[0][3]).toEqual({ where: 'type == "index"', limit: 255 });
  });

  it("uma query por TTL", async () => {
    const gateway = gatewayWith(async () => ({ results: [{ path: "a.md", frontmatter: {} }] }));
    let t = 1000;
    const cache = new IndexListCache(asGateway(gateway), 300_000, () => t);
    await cache.list("set-1", CONN);
    await cache.list("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(1);
    t += 300_001;
    await cache.list("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(2);
  });

  it("coleção sem indexes dá lista vazia", async () => {
    const cache = new IndexListCache(asGateway(gatewayWith(async () => ({ results: [] }))));
    expect(await cache.list("set-1", CONN)).toEqual([]);
  });

  it("mais de 255 vira 255", async () => {
    const results = Array.from({ length: 300 }, (_, i) => ({ path: `n/${String(i).padStart(3, "0")}.md`, frontmatter: {} }));
    const cache = new IndexListCache(asGateway(gatewayWith(async () => ({ results }))));
    expect(await cache.list("set-1", CONN)).toHaveLength(255);
  });

  it("sem query ou com falha: vazio, e falha em cache negativo de 30 s", async () => {
    const noQuery = gatewayWith(async () => ({ results: [] }), ["read"]);
    expect(await new IndexListCache(asGateway(noQuery)).list("set-1", CONN)).toEqual([]);
    expect(queryCalls(noQuery)).toHaveLength(0);

    const failing = gatewayWith(async () => { throw new Error("boom"); });
    let t = 0;
    const cache = new IndexListCache(asGateway(failing), 300_000, () => t);
    expect(await cache.list("set-1", CONN)).toEqual([]);
    expect(await cache.list("set-1", CONN)).toEqual([]);
    expect(queryCalls(failing)).toHaveLength(1);
    t += 30_001;
    await cache.list("set-1", CONN);
    expect(queryCalls(failing)).toHaveLength(2);
  });

  it("peek: null no miss frio, lista depois; uma única atualização em voo", async () => {
    const gateway = gatewayWith(async () => ({ results: [{ path: "a.md", frontmatter: { title: "A" } }] }));
    const cache = new IndexListCache(asGateway(gateway));
    expect(cache.peek("set-1", CONN)).toBeNull();
    expect(cache.peek("set-1", CONN)).toBeNull();
    await cache.list("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(1);
    expect(cache.peek("set-1", CONN)).toEqual([{ path: "a.md", title: "A", description: null }]);
  });

  it("peek devolve a lista vencida e atualiza em segundo plano", async () => {
    const gateway = gatewayWith(async () => ({ results: [{ path: "a.md", frontmatter: {} }] }));
    let t = 0;
    const cache = new IndexListCache(asGateway(gateway), 1000, () => t);
    await cache.list("set-1", CONN);
    t += 2000;
    expect(cache.peek("set-1", CONN)).toHaveLength(1);
    await vi.waitFor(() => expect(queryCalls(gateway)).toHaveLength(2));
  });
});
