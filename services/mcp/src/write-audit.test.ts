import { describe, expect, it, vi } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { ConnectGateway } from "./connect.js";
import { GatewayOperationError } from "./errors.js";
import { createMcpServer } from "./mcp.js";
import type { OAuthService } from "./oauth.js";
import { createDatabase } from "./db.js";
import { createPgWriteAudit, type WriteAudit, type WriteAuditEntry } from "./write-audit.js";

const CONN = "01911111-1111-7111-8111-111111111111";
const context = { clientId: "client-enola", connectionSetId: "set-1", scopes: ["mdbase:read", "mdbase:write"] as const };

async function connect(gateway: Partial<ConnectGateway>, audit: WriteAudit) {
  const server = createMcpServer(
    { ...context, scopes: [...context.scopes] },
    gateway as ConnectGateway,
    {} as unknown as OAuthService,
    audit
  );
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test", version: "1" });
  await Promise.all([server.connect(serverSide), client.connect(clientSide)]);
  return client;
}

function fakeAudit(overrides: Partial<WriteAudit> = {}) {
  return {
    begin: vi.fn(async () => "42"),
    finish: vi.fn(async () => undefined),
    ...overrides
  } satisfies WriteAudit as WriteAudit & { begin: ReturnType<typeof vi.fn>; finish: ReturnType<typeof vi.fn> };
}

describe("write audit", () => {
  it("update_record bem-sucedido registra begin e finish ok", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r2" })) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    const result = await client.callTool({
      name: "update_record",
      arguments: { connection_id: CONN, path: "00-conhecimento/backlog-conteudo.md", body: "novo" }
    });

    expect(result.isError).toBeFalsy();
    expect(audit.begin).toHaveBeenCalledWith(expect.objectContaining({
      clientId: "client-enola",
      connectionSetId: "set-1",
      connectionId: CONN,
      tool: "update_record",
      path: "00-conhecimento/backlog-conteudo.md",
      toPath: null
    }));
    expect(audit.finish).toHaveBeenCalledWith("42", { ok: true, revision: "r2" });
    await client.close();
  });

  it("escrita que falha fecha como error", async () => {
    const gateway = {
      operation: vi.fn(async () => { throw new GatewayOperationError("revision_conflict", "Revisão mudou."); })
    };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    const result = await client.callTool({
      name: "update_record",
      arguments: { connection_id: CONN, path: "a.md", body: "x", if_revision: "r1" }
    });

    expect(result.isError).toBe(true);
    expect(audit.finish).toHaveBeenCalledWith("42", { ok: false, errorCode: "revision_conflict" });
    await client.close();
  });

  it("falha sem código do gateway fecha como operation_failed", async () => {
    const gateway = { operation: vi.fn(async () => { throw new Error("boom"); }) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({ name: "delete_record", arguments: { connection_id: CONN, path: "a.md" } });

    expect(audit.finish).toHaveBeenCalledWith("42", { ok: false, errorCode: "operation_failed" });
    await client.close();
  });

  it("rename registra from e to", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r3" })) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({ name: "rename_record", arguments: { connection_id: CONN, from: "a.md", to: "b.md" } });

    expect(audit.begin).toHaveBeenCalledWith(expect.objectContaining({ tool: "rename_record", path: "a.md", toPath: "b.md" }));
    expect(audit.finish).toHaveBeenCalledWith("42", { ok: true, revision: "r3" });
    await client.close();
  });

  it("create_record sem path registra path nulo", async () => {
    const gateway = { operation: vi.fn(async () => ({})) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({ name: "create_record", arguments: { connection_id: CONN, type: "nota" } });

    expect(audit.begin).toHaveBeenCalledWith(expect.objectContaining({ tool: "create_record", path: null, toPath: null }));
    expect(audit.finish).toHaveBeenCalledWith("42", { ok: true, revision: null });
    await client.close();
  });

  it("update_type registra path ou, na falta dele, name", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r4" })) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({
      name: "update_type",
      arguments: { connection_id: CONN, name: "tarefa", document: "type: tarefa", if_revision: "r3" }
    });

    expect(audit.begin).toHaveBeenCalledWith(expect.objectContaining({ tool: "update_type", path: "tarefa", toPath: null }));
    await client.close();
  });

  it("begin que falha impede a escrita", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r2" })) };
    const audit = fakeAudit({ begin: vi.fn(async () => { throw new Error("banco fora"); }) });
    const client = await connect(gateway, audit);

    const result = await client.callTool({
      name: "update_record",
      arguments: { connection_id: CONN, path: "a.md", body: "x" }
    });

    expect(gateway.operation).not.toHaveBeenCalled();
    expect(audit.finish).not.toHaveBeenCalled();
    expect(result.isError).toBe(true);
    expect(JSON.parse((result.content as Array<{ text: string }>)[0].text).error.code).toBe("audit_unavailable");
    await client.close();
  });

  it("registro não guarda conteúdo", async () => {
    const gateway = { operation: vi.fn(async () => ({ revision: "r2" })) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({
      name: "create_record",
      arguments: { connection_id: CONN, path: "a.md", frontmatter: { titulo: "segredo" }, body: "conteúdo" }
    });

    const entry = audit.begin.mock.calls[0][0] as WriteAuditEntry;
    expect(Object.keys(entry).sort()).toEqual([
      "clientId", "connectionId", "connectionSetId", "path", "requestId", "toPath", "tool"
    ]);
    await client.close();
  });

  it("ferramenta de leitura não registra", async () => {
    const gateway = { operation: vi.fn(async () => ({ path: "a.md" })) };
    const audit = fakeAudit();
    const client = await connect(gateway, audit);

    await client.callTool({ name: "read_record", arguments: { connection_id: CONN, path: "a.md" } });

    expect(audit.begin).not.toHaveBeenCalled();
    await client.close();
  });
});

describe("createPgWriteAudit", () => {
  it("grava a linha com o nome do cliente e fecha com o desfecho", async () => {
    const db = await createDatabase("memory");
    await db.query("INSERT INTO mcp_clients (id, client_name, redirect_uris) VALUES ('client-enola', 'Enola', '[]')");
    const audit = createPgWriteAudit(db);

    const id = await audit.begin({ clientId: "client-enola", connectionSetId: "set-1", connectionId: CONN, tool: "rename_record", path: "a.md", toPath: "b.md", requestId: "req-1" });
    await audit.finish(id, { ok: true, revision: "r9" });

    const { rows } = await db.query("SELECT client_name, tool, path, to_path, outcome, error_code, revision, finished_at IS NOT NULL AS finished FROM mcp_write_audit WHERE id = $1", [id]);
    expect(rows).toEqual([{ client_name: "Enola", tool: "rename_record", path: "a.md", to_path: "b.md", outcome: "ok", error_code: null, revision: "r9", finished: true }]);
    await db.end();
  });

  it("grava mesmo quando o cliente já não existe, com client_name nulo", async () => {
    const db = await createDatabase("memory");
    const audit = createPgWriteAudit(db);

    const id = await audit.begin({ clientId: "client-gone", connectionSetId: "set-1", connectionId: CONN, tool: "delete_record", path: "a.md", toPath: null, requestId: "req-2" });
    await audit.finish(id, { ok: false, errorCode: "revision_conflict" });

    const { rows } = await db.query("SELECT client_name, outcome, error_code FROM mcp_write_audit WHERE id = $1", [id]);
    expect(rows).toEqual([{ client_name: null, outcome: "error", error_code: "revision_conflict" }]);
    await db.end();
  });
});
