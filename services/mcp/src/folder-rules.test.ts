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
    expect(queryCalls(gateway)[0][3]).toEqual({ where: 'file.name == "AGENTS.md"' });
    t += 300_001;
    await cache.rulePaths("set-1", CONN);
    expect(queryCalls(gateway)).toHaveLength(2);
  });
  it("falha da query dá [] e não é guardada", async () => {
    let fail = true;
    const gateway = gatewayWith(async () => { if (fail) throw new Error("boom"); return okQuery(); });
    const cache = new FolderRulesCache(gateway as unknown as ConnectGateway);
    expect(await cache.rulePaths("set-1", CONN)).toEqual([]);
    fail = false;
    expect(await cache.rulePaths("set-1", CONN)).toEqual(RULES);
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
  it("acrescenta a regra mais próxima à resposta", async () => {
    const result = await read(await connect(gatewayWith(okQuery)));
    expect(result.structuredContent).toEqual({
      path: "13-engenharia/x.md", revision: "r1", frontmatter: {}, body: "b", folder_rule: "13-engenharia/AGENTS.md"
    });
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
