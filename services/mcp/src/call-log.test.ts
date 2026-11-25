import { describe, expect, it } from "vitest";
import { createDatabase } from "./db.js";
import { createPgCallLog, noCallLog, type CallLogEntry } from "./call-log.js";

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

describe("noCallLog", () => {
  it("não grava nada", async () => {
    await expect(noCallLog.record(entry())).resolves.toBeUndefined();
    await expect(noCallLog.purgeOlderThan(30)).resolves.toBe(0);
  });
});
