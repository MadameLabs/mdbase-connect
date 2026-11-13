import type { DatabaseQueryable } from "./db.js";

export type WriteTool = "create_record" | "update_record" | "rename_record" | "delete_record" | "create_type" | "update_type";

export interface WriteAuditEntry {
  clientId: string;
  connectionSetId: string;
  connectionId: string;
  tool: WriteTool;
  path: string | null;
  toPath: string | null;
  requestId: string;
}

export type WriteOutcome = { ok: true; revision: string | null } | { ok: false; errorCode: string };

export interface WriteAudit {
  begin(entry: WriteAuditEntry): Promise<string>;
  finish(id: string, result: WriteOutcome): Promise<void>;
}

export function createPgWriteAudit(db: DatabaseQueryable): WriteAudit {
  return {
    async begin(entry) {
      const { rows } = await db.query<{ id: string }>(
        `INSERT INTO mcp_write_audit (client_id, client_name, connection_set_id, connection_id, tool, path, to_path, request_id)
         SELECT $1::text, c.client_name, $2::text, $3::text, $4::text, $5::text, $6::text, $7::text
         FROM (SELECT 1 AS one) AS d
         LEFT JOIN mcp_clients c ON c.id = $1::text
         RETURNING id`,
        [entry.clientId, entry.connectionSetId, entry.connectionId, entry.tool, entry.path, entry.toPath, entry.requestId]
      );
      return String(rows[0].id);
    },
    async finish(id, result) {
      await db.query(
        `UPDATE mcp_write_audit
         SET outcome = $2, error_code = $3, revision = $4, finished_at = now()
         WHERE id = $1`,
        [id, result.ok ? "ok" : "error", result.ok ? null : result.errorCode, result.ok ? result.revision : null]
      );
    }
  };
}
