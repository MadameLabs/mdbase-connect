import { describe, expect, it, vi } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { ConnectGateway } from "./connect.js";
import { FolderRulesCache, nearestFolderRule } from "./folder-rules.js";
import { createMcpServer } from "./mcp.js";
import type { OAuthService } from "./oauth.js";

const CONN = "01911111-1111-7111-8111-111111111111";
const ctx = { clientId: "c", connectionSetId: "set-1", scopes: ["mdbase:read"] as ("mdbase:read" | "mdbase:write")[] };
const RULES = ["AGENTS.md", "13-engenharia/AGENTS.md"];

describe("nearestFolderRule", () => {
  it("acha o AGENTS.md mais próximo acima da nota", () => {
    expect(nearestFolderRule("13-engenharia/01-planejamento/x.md", RULES)).toBe("13-engenharia/AGENTS.md");
    expect(nearestFolderRule("13-engenharia/01-planejamento/x.md", [...RULES, "13-engenharia/01-planejamento/AGENTS.md"]))
      .toBe("13-engenharia/01-planejamento/AGENTS.md");
  });
  it("nota na raiz ou sem regra além da raiz dá null", () => {
    expect(nearestFolderRule("x.md", RULES)).toBeNull();
    expect(nearestFolderRule("02-marcas/x.md", RULES)).toBeNull();
  });
  it("um AGENTS.md de pasta começa um nível acima", () => {
    expect(nearestFolderRule("13-engenharia/AGENTS.md", RULES)).toBeNull();
    expect(nearestFolderRule("13-engenharia/01-planejamento/AGENTS.md", [...RULES, "13-engenharia/01-planejamento/AGENTS.md"]))
      .toBe("13-engenharia/AGENTS.md");
    expect(nearestFolderRule("AGENTS.md", RULES)).toBeNull();
  });
  it("prefixo parecido não conta", () => {
    expect(nearestFolderRule("13-engenharia-velho/x.md", RULES)).toBeNull();
  });
});

const connections = (operations = ["read", "query"]) => async () =>
  [{ id: CONN, collection_id: "c", display_name: "x", operations, scope: {}, authority: "local" }];

function gatewayWith(queryImpl: () => Promise<unknown>, operations?: string[]) {
  const operation = vi.fn(async (_s: string, _c: string, op: string) => {
    if (op === "query") return queryImpl();
    if (op === "read") return { path: "13-engenharia/x.md", revision: "r1", frontmatter: {}, body: "b" };
    throw new Error(`unexpected ${op}`);
  });
  return { operation, listConnections: vi.fn(connections(operations)) };
}
const okQuery = async () => ({ results: [{ path: "AGENTS.md" }, { path: "13-engenharia/AGENTS.md" }] });
const queryCalls = (g: ReturnType<typeof gatewayWith>) => (g.operation.mock.calls as unknown[][]).filter((c) => c[2] === "query");

describe("FolderRulesCache", () => {
  it("uma query para duas leituras seguidas e outra depois do TTL", async () => {
    const gateway = gatewayWith(okQuery);
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    expect(await cache.rulePaths("set-1", CONN)).toEqual(RULES);
    expect(await cache.rulePaths("set-1", CONN)).toEqual(RULES);
    expect(queryCalls(gateway)).toHaveLength(1);
    expect(queryCalls(gateway)[0][3]).toEqual({ where: 'file.name == "AGENTS.md"', limit: 1000 });
    t += 300_001;
    await cache.rulePaths("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("falha da query dá [] e só repete depois de 30 s", async () => {
    let fail = true;
    const gateway = gatewayWith(async () => { if (fail) throw new Error("boom"); return okQuery(); });
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    expect(await cache.rulePaths("set-1", CONN)).toEqual([]);
    fail = false;
    t += 29_000;
    expect(await cache.rulePaths("set-1", CONN)).toEqual([]);
    expect(queryCalls(gateway)).toHaveLength(1);
    t += 1_001;
    expect(await cache.rulePaths("set-1", CONN)).toEqual(RULES);
  });
  it("pede limit 1000 e resultado truncado devolve os caminhos sem guardar como completo", async () => {
    const gateway = gatewayWith(async () => ({ results: [{ path: "AGENTS.md" }], meta: { has_more: true } }));
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    expect(await cache.rulePaths("set-1", CONN)).toEqual(["AGENTS.md"]);
    expect(queryCalls(gateway)[0][3]).toEqual({ where: 'file.name == "AGENTS.md"', limit: 1000 });
    expect(await cache.rulePaths("set-1", CONN)).toEqual(["AGENTS.md"]);
    expect(queryCalls(gateway)).toHaveLength(1);
    t += 30_001; // truncated pages live only as long as a failure, not the 5 min TTL
    await cache.rulePaths("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("cursor na resposta também conta como truncado", async () => {
    const gateway = gatewayWith(async () => ({ results: [{ path: "AGENTS.md" }], meta: { cursor: "c1" } }));
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    await cache.rulePaths("set-1", CONN);
    t += 30_001;
    await cache.rulePaths("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("peek: miss frio devolve null e inicia uma única atualização; depois devolve a lista", async () => {
    const gateway = gatewayWith(okQuery);
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway);
    expect(cache.peek("set-1", CONN)).toBeNull();
    expect(cache.peek("set-1", CONN)).toBeNull();
    await vi.waitFor(() => expect(cache.peek("set-1", CONN)).toEqual(RULES));
    expect(queryCalls(gateway)).toHaveLength(1);
  });
  it("peek: entrada vencida é servida velha enquanto atualiza", async () => {
    let paths = ["AGENTS.md"];
    const gateway = gatewayWith(async () => ({ results: paths.map((path) => ({ path })) }));
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    await cache.rulePaths("set-1", CONN);
    paths = RULES;
    t += 300_001;
    expect(cache.peek("set-1", CONN)).toEqual(["AGENTS.md"]);
    await vi.waitFor(() => expect(cache.peek("set-1", CONN)).toEqual(RULES));
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("peek: falha não é repetida em 30 s e a lista velha sobrevive à falha", async () => {
    let fail = false;
    const gateway = gatewayWith(async () => { if (fail) throw new Error("boom"); return okQuery(); });
    let t = 1000;
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway, 300_000, () => t);
    await cache.rulePaths("set-1", CONN);
    fail = true;
    t += 300_001;
    expect(cache.peek("set-1", CONN)).toEqual(RULES);
    await vi.waitFor(() => expect(queryCalls(gateway)).toHaveLength(2));
    await new Promise((r) => setTimeout(r, 5));
    t += 29_000;
    expect(cache.peek("set-1", CONN)).toEqual(RULES);
    await new Promise((r) => setTimeout(r, 5));
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("peek: falha em miss frio não gera tempestade de queries", async () => {
    const gateway = gatewayWith(async () => { throw new Error("boom"); });
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway);
    expect(cache.peek("set-1", CONN)).toBeNull();
    await vi.waitFor(() => expect(queryCalls(gateway)).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 5));
    expect(cache.peek("set-1", CONN)).toBeNull();
    await new Promise((r) => setTimeout(r, 5));
    expect(queryCalls(gateway)).toHaveLength(1);
  });
  it("misses concorrentes (peek e rulePaths) fazem uma query só", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    const gateway = gatewayWith(async () => { await gate; return okQuery(); });
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway);
    cache.peek("set-1", CONN);
    const a = cache.rulePaths("set-1", CONN);
    const b = cache.rulePaths("set-1", CONN);
    cache.peek("set-1", CONN);
    release();
    expect(await a).toEqual(RULES);
    expect(await b).toEqual(RULES);
    expect(queryCalls(gateway)).toHaveLength(1);
  });
  it("conexão sem query dá [] sem consultar", async () => {
    const gateway = gatewayWith(okQuery, ["read"]);
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway);
    expect(await cache.rulePaths("set-1", CONN)).toEqual([]);
    expect(queryCalls(gateway)).toHaveLength(0);
  });
});

async function connect(gateway: ReturnType<typeof gatewayWith>, withCache = true) {
  const g = gateway as unknown as ConnectGateway;
  const server = createMcpServer(ctx, g, {} as OAuthService, { begin: async () => "1", finish: async () => undefined },
    withCache ? { folderRules: new FolderRulesCache(g) } : {});
  const [a, b] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test", version: "1" });
  await Promise.all([server.connect(b), client.connect(a)]);
  return client;
}
const read = (c: Client) => c.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "13-engenharia/x.md" } });

describe("folder_rule no read_record", () => {
  it("miss frio devolve null sem esperar a query e a leitura seguinte traz a regra", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    const gateway = gatewayWith(async () => { await gate; return okQuery(); });
    const client = await connect(gateway);
    const first = await read(client); // would hang if read_record waited on the query
    expect(first.structuredContent).toMatchObject({ revision: "r1", folder_rule: null });
    await vi.waitFor(() => expect(queryCalls(gateway)).toHaveLength(1));
    release();
    await vi.waitFor(async () => {
      expect((await read(client)).structuredContent).toMatchObject({ folder_rule: "13-engenharia/AGENTS.md" });
    });
    expect(queryCalls(gateway)).toHaveLength(1);
  });
  it("falha da query dá folder_rule null sem afetar a leitura", async () => {
    const result = await read(await connect(gatewayWith(async () => { throw new Error("boom"); })));
    expect(result.isError).toBeFalsy();
    expect(result.structuredContent).toMatchObject({ revision: "r1", folder_rule: null });
  });
  it("sem cache configurado devolve null sem query", async () => {
    const gateway = gatewayWith(okQuery);
    const result = await read(await connect(gateway, false));
    expect(result.structuredContent).toMatchObject({ folder_rule: null });
    expect(queryCalls(gateway)).toHaveLength(0);
  });
  it("erro de leitura continua igual, sem folder_rule", async () => {
    const gateway = gatewayWith(okQuery);
    gateway.operation.mockImplementation(async (_s: string, _c: string, op: string) => {
      if (op === "read") throw new Error("x");
      return { results: [] };
    });
    const result = await read(await connect(gateway));
    expect(result.isError).toBe(true);
    expect(result.structuredContent).not.toHaveProperty("folder_rule");
  });
});
