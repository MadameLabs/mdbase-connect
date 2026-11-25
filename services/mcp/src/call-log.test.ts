import { describe, expect, it, vi } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { createDatabase } from "./db.js";
import { createPgCallLog, noCallLog, type CallLog, type CallLogEntry } from "./call-log.js";
import type { ConnectGateway } from "./connect.js";
import { GatewayOperationError } from "./errors.js";
import { createMcpServer } from "./mcp.js";
import type { OAuthService } from "./oauth.js";
import { runtimeConfigFromEnv } from "./config.js";

function entry(overrides: Partial<CallLogEntry> = {}): CallLogEntry {
  return {
    clientId: "client-enola",
    connectionSetId: "set-1",
    connectionId: "conn-1",
    collectionId: "col-1",
    tool: "query_records",
    durationMs: 12,
    outcome: "ok",
    errorCode: null,
    resultCount: 3,
    paths: ["a.md", "b.md"],
    queryText: "backlog",
    ranking: "bm25",
    ...overrides
  };
}

describe("createPgCallLog", () => {
  it("grava a linha com o nome do cliente copiado de mcp_clients", async () => {
    const db = await createDatabase("memory");
    await db.query("INSERT INTO mcp_clients (id, client_name, redirect_uris) VALUES ('client-enola', 'Enola', '[]')");
    const log = createPgCallLog(db);

    await log.record(entry());

    const { rows } = await db.query(
      "SELECT client_id, client_name, connection_set_id, connection_id, collection_id, tool, duration_ms, outcome, error_code, result_count, paths, query_text, ranking FROM mcp_call_log"
    );
    expect(rows).toEqual([{
      client_id: "client-enola", client_name: "Enola", connection_set_id: "set-1", connection_id: "conn-1",
      collection_id: "col-1", tool: "query_records", duration_ms: 12, outcome: "ok", error_code: null,
      result_count: 3, paths: ["a.md", "b.md"], query_text: "backlog", ranking: "bm25"
    }]);
    await db.end();
  });

  it("corta paths em 50 itens", async () => {
    const db = await createDatabase("memory");
    const log = createPgCallLog(db);

    await log.record(entry({ paths: Array.from({ length: 80 }, (_, i) => `n${i}.md`) }));

    const { rows } = await db.query<{ paths: string[] }>("SELECT paths FROM mcp_call_log");
    expect(rows[0].paths).toHaveLength(50);
    expect(rows[0].paths[49]).toBe("n49.md");
    await db.end();
  });

  it("purgeOlderThan apaga as de 31 dias, mantém as de 29 e devolve a contagem", async () => {
    const db = await createDatabase("memory");
    const log = createPgCallLog(db);
    await log.record(entry({ tool: "velha" }));
    await log.record(entry({ tool: "nova" }));
    await db.query("UPDATE mcp_call_log SET created_at = now() - interval '31 days' WHERE tool = 'velha'");
    await db.query("UPDATE mcp_call_log SET created_at = now() - interval '29 days' WHERE tool = 'nova'");

    const removed = await log.purgeOlderThan(30);

    expect(removed).toBe(1);
    const { rows } = await db.query("SELECT tool FROM mcp_call_log");
    expect(rows).toEqual([{ tool: "nova" }]);
    await db.end();
  });
});

describe("query_text", () => {
  it("corta query_text em 2.000 caracteres", async () => {
    const db = await createDatabase("memory");
    const log = createPgCallLog(db);

    await log.record(entry({ queryText: "x".repeat(5000) }));

    const { rows } = await db.query<{ query_text: string }>("SELECT query_text FROM mcp_call_log");
    expect(rows[0].query_text).toHaveLength(2000);
    await db.end();
  });
});

describe("noCallLog", () => {
  it("não grava nada", async () => {
    await expect(noCallLog.record(entry())).resolves.toBeUndefined();
    await expect(noCallLog.purgeOlderThan(30)).resolves.toBe(0);
  });
});

const CONN = "01911111-1111-7111-8111-111111111111";
const ctx = { clientId: "client-enola", connectionSetId: "set-1", scopes: ["mdbase:read", "mdbase:write"] as ("mdbase:read" | "mdbase:write")[] };

function fakeLog(record?: CallLog["record"]) {
  const entries: CallLogEntry[] = [];
  const log = {
    record: vi.fn(record ?? (async (e: CallLogEntry) => { entries.push(e); })),
    purgeOlderThan: vi.fn(async () => 0)
  };
  return { log: log as CallLog & typeof log, entries };
}

async function connectServer(gateway: unknown, callLog: CallLog, queryTextCollections?: ReadonlySet<string>) {
  const server = createMcpServer(
    ctx, gateway as ConnectGateway, {} as unknown as OAuthService,
    { begin: async () => "1", finish: async () => undefined },
    { callLog, queryTextCollections }
  );
  const [a, b] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test", version: "1" });
  await Promise.all([server.connect(b), client.connect(a)]);
  return client;
}

const settled = (entries: unknown[]) => vi.waitFor(() => expect(entries.length).toBeGreaterThan(0));
const connections = async () => [{ id: CONN, collection_id: "col-1", display_name: "x", operations: [], scope: {}, authority: "local" }];

describe("registro das chamadas no servidor MCP", () => {
  it("read_record ok grava tool, caminho, contagem 1 e tempo", async () => {
    const gateway = { operation: vi.fn(async () => ({ path: "a/b.md", revision: "r1", frontmatter: { segredo: "x" }, body: "corpo secreto" })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "a/b.md" } });
    await settled(entries);
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      clientId: "client-enola", connectionSetId: "set-1", connectionId: CONN, collectionId: "col-1",
      tool: "read_record", outcome: "ok", errorCode: null, resultCount: 1, paths: ["a/b.md"], queryText: null, ranking: null
    });
    expect(entries[0]!.durationMs).toBeGreaterThanOrEqual(0);
    expect(JSON.stringify(entries[0])).not.toMatch(/corpo secreto|segredo/);
    await client.close();
  });

  it("query_records com 3 resultados grava contagem e os 3 caminhos", async () => {
    const results = ["a.md", "b.md", "c.md"].map((path) => ({ path, frontmatter: { t: 1 }, body: "corpo secreto" }));
    const gateway = { operation: vi.fn(async () => ({ results, meta: {} })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "query_records", arguments: { connection_id: CONN, where: 'title == "x"' } });
    await settled(entries);
    expect(entries[0]).toMatchObject({ tool: "query_records", resultCount: 3, paths: ["a.md", "b.md", "c.md"] });
    expect(JSON.stringify(entries[0])).not.toContain("corpo secreto");
    await client.close();
  });

  it("grava o where só em coleção da lista queryTextCollections", async () => {
    const gateway = { operation: vi.fn(async () => ({ results: [] })), listConnections: vi.fn(connections) };
    const where = 'title.contains("x")';
    const inList = fakeLog();
    const c1 = await connectServer(gateway, inList.log, new Set(["col-1"]));
    await c1.callTool({ name: "query_records", arguments: { connection_id: CONN, where } });
    await settled(inList.entries);
    expect(inList.entries[0]!.queryText).toBe(where);
    const outList = fakeLog();
    const c2 = await connectServer(gateway, outList.log, new Set(["outra"]));
    await c2.callTool({ name: "query_records", arguments: { connection_id: CONN, where } });
    await settled(outList.entries);
    expect(outList.entries[0]!.queryText).toBeNull();
    const noWhere = fakeLog();
    const c3 = await connectServer(gateway, noWhere.log, new Set(["col-1"]));
    await c3.callTool({ name: "query_records", arguments: { connection_id: CONN } });
    await settled(outList.entries);
    expect(noWhere.entries[0]!.queryText).toBeNull();
  });

  it("erro do gateway grava outcome error e o code", async () => {
    const gateway = {
      operation: vi.fn(async () => { throw new GatewayOperationError("insufficient_collection_access", "negado"); }),
      listConnections: vi.fn(connections)
    };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    const result = await client.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "a.md" } });
    await settled(entries);
    expect(result.isError).toBe(true);
    expect(entries[0]).toMatchObject({ outcome: "error", errorCode: "insufficient_collection_access", resultCount: null });
  });

  it("falha do callLog.record não muda a resposta e vai ao console", async () => {
    const value = { path: "a.md", revision: "r1", frontmatter: {}, body: "x" };
    const gateway = { operation: vi.fn(async () => value), listConnections: vi.fn(connections) };
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { log } = fakeLog(async () => { throw new Error("banco fora"); });
    const client = await connectServer(gateway, log);
    const result = await client.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "a.md" } });
    await vi.waitFor(() => expect(spy).toHaveBeenCalled());
    expect(result.isError).toBeFalsy();
    expect(result.structuredContent).toEqual(value);
    expect(spy).toHaveBeenCalled();
    spy.mockRestore();
  });

  it("a resposta não espera a gravação", async () => {
    const gateway = { operation: vi.fn(async () => ({ path: "a.md" })), listConnections: vi.fn(connections) };
    const { log } = fakeLog(() => new Promise<void>(() => undefined));
    const client = await connectServer(gateway, log);
    const result = await client.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "a.md" } });
    expect(result.isError).toBeFalsy();
  });

  it("escrita entra no registro com o caminho gravado, sem o corpo", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r2", path: "n.md" })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "update_record", arguments: { connection_id: CONN, path: "n.md", body: "corpo secreto" } });
    await settled(entries);
    expect(entries[0]).toMatchObject({ tool: "update_record", outcome: "ok", paths: ["n.md"] });
    expect(JSON.stringify(entries[0])).not.toContain("corpo secreto");
  });
});

describe("registro de tipos", () => {
  it("read_type por nome grava paths vazio, não o nome do tipo", async () => {
    const gateway = { operation: vi.fn(async () => ({ name: "tarefa", revision: "r1" })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "read_type", arguments: { connection_id: CONN, name: "tarefa" } });
    await settled(entries);
    expect(entries[0]).toMatchObject({ tool: "read_type", paths: [] });
  });

  it("update_type por nome grava paths vazio", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r2" })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "update_type", arguments: { connection_id: CONN, name: "tarefa", document: "x", if_revision: "r1" } });
    await settled(entries);
    expect(entries[0]).toMatchObject({ tool: "update_type", paths: [] });
  });

  it("read_type com caminho grava esse caminho", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r1" })), listConnections: vi.fn(connections) };
    const { log, entries } = fakeLog();
    const client = await connectServer(gateway, log);
    await client.callTool({ name: "read_type", arguments: { connection_id: CONN, path: "_types/tarefa.md" } });
    await settled(entries);
    expect(entries[0]).toMatchObject({ paths: ["_types/tarefa.md"] });
  });
});

describe("configuração do registro", () => {
  const base = { MDBASE_MCP_MASTER_KEY: "k".repeat(40) };
  it("usa padrões 30 dias e lista vazia", () => {
    const config = runtimeConfigFromEnv(base);
    expect(config.callLogRetentionDays).toBe(30);
    expect(config.queryTextCollections.size).toBe(0);
  });
  it("lê dias e coleções, e rejeita valor fora da faixa", () => {
    const config = runtimeConfigFromEnv({ ...base, MDBASE_MCP_CALL_LOG_RETENTION_DAYS: "7", MDBASE_MCP_CALL_LOG_QUERY_TEXT_COLLECTIONS: " a, b ,," });
    expect(config.callLogRetentionDays).toBe(7);
    expect([...config.queryTextCollections]).toEqual(["a", "b"]);
    expect(() => runtimeConfigFromEnv({ ...base, MDBASE_MCP_CALL_LOG_RETENTION_DAYS: "0" })).toThrow();
    expect(() => runtimeConfigFromEnv({ ...base, MDBASE_MCP_CALL_LOG_RETENTION_DAYS: "366" })).toThrow();
  });
});
