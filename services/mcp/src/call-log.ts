import type { DatabaseQueryable } from "./db.js";

const MAX_PATHS = 50;
const MAX_QUERY_TEXT = 2000;

// Só metadados: nunca o conteúdo das notas.
export interface CallLogEntry {
  clientId: string;
  connectionSetId: string;
  connectionId: string | null;
  collectionId: string | null;
  tool: string;
  durationMs: number;
  outcome: "ok" | "error";
  errorCode: string | null;
  resultCount: number | null;
  paths: string[];
  queryText: string | null;
  ranking: string | null;
}

export interface CallLog {
  record(entry: CallLogEntry): Promise<void>;
  purgeOlderThan(days: number): Promise<number>;
}

// Padrão quando ninguém passa um registro: não grava nada.
export const noCallLog: CallLog = {
  async record() {},
  async purgeOlderThan() {
    return 0;
  }
};

export function createPgCallLog(db: DatabaseQueryable): CallLog {
  return {
    async record(entry) {
      await db.query(
        `INSERT INTO mcp_call_log (client_id, client_name, connection_set_id, connection_id, collection_id, tool, duration_ms, outcome, error_code, result_count, paths, query_text, ranking)
         SELECT $1::text, c.client_name, $2::text, $3::text, $4::text, $5::text, $6::integer, $7::text, $8::text, $9::integer, $10::text[], $11::text, $12::text
         FROM (SELECT 1 AS one) AS d
         LEFT JOIN mcp_clients c ON c.id = $1::text`,
        [
          entry.clientId, entry.connectionSetId, entry.connectionId, entry.collectionId, entry.tool,
          entry.durationMs, entry.outcome, entry.errorCode, entry.resultCount,
          entry.paths.slice(0, MAX_PATHS), entry.queryText?.slice(0, MAX_QUERY_TEXT) ?? null, entry.ranking
        ]
      );
    },
    async purgeOlderThan(days) {
      const result = await db.query(
        `DELETE FROM mcp_call_log WHERE created_at < $1::timestamptz`,
        [new Date(Date.now() - days * 86_400_000).toISOString()]
      );
      return result.rowCount ?? 0;
    }
  };
}
