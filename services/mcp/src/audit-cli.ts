import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { createDatabase, type DatabaseQueryable } from "./db.js";

export interface AuditFilters {
  cliente?: string;
  nota?: string;
  desde?: string;
  ate?: string;
  limite?: number;
}

const LIMITE_PADRAO = 50;
const LIMITE_MAXIMO = 1000;

function isDataSimples(valor: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(valor);
}

export function buildAuditQuery(filters: AuditFilters): { text: string; values: unknown[] } {
  const conditions: string[] = [];
  const values: unknown[] = [];
  const add = (value: unknown) => {
    values.push(value);
    return `$${values.length}`;
  };

  if (filters.cliente !== undefined) {
    conditions.push(`client_name = ${add(filters.cliente)}`);
  }
  if (filters.nota !== undefined) {
    const padrao = `%${filters.nota.replace(/[\\%_]/g, "\\$&")}%`;
    const ref = add(padrao);
    conditions.push(`(path ILIKE ${ref} OR to_path ILIKE ${ref})`);
  }
  if (filters.desde !== undefined) {
    conditions.push(`created_at >= ${add(filters.desde)}`);
  }
  if (filters.ate !== undefined) {
    const ref = add(filters.ate);
    conditions.push(
      isDataSimples(filters.ate)
        ? `created_at < (${ref}::date + interval '1 day')::timestamptz`
        : `created_at <= ${ref}`
    );
  }

  const limite = Math.min(Math.max(Math.trunc(filters.limite ?? LIMITE_PADRAO), 1), LIMITE_MAXIMO);
  const where = conditions.length > 0 ? `WHERE ${conditions.join(" AND ")}` : "";
  const text = `SELECT created_at, client_name, tool, path, to_path, outcome, error_code, revision
FROM mcp_write_audit
${where}
ORDER BY created_at DESC LIMIT ${add(limite)}`;

  return { text, values };
}

const COLUNAS = ["created_at", "client_name", "tool", "path", "to_path", "outcome", "error_code", "revision"] as const;

function celula(valor: unknown): string {
  if (valor === null || valor === undefined) return "";
  const texto = valor instanceof Date ? valor.toISOString() : String(valor);
  return texto.replace(/[\t\r\n]/g, " ");
}

export function formatAuditTsv(rows: Record<string, unknown>[]): string {
  const linhas = rows.map((row) => COLUNAS.map((coluna) => celula(row[coluna])).join("\t"));
  return [COLUNAS.join("\t"), ...linhas].join("\n") + "\n";
}

export async function runAuditQuery(db: DatabaseQueryable, filters: AuditFilters): Promise<string> {
  const query = buildAuditQuery(filters);
  const { rows } = await db.query(query.text, query.values);
  return formatAuditTsv(rows);
}

function lerFiltros(argv: string[]): AuditFilters {
  const { values } = parseArgs({
    args: argv,
    options: {
      cliente: { type: "string" },
      nota: { type: "string" },
      desde: { type: "string" },
      ate: { type: "string" },
      limite: { type: "string" }
    },
    strict: true
  });
  const filtros: AuditFilters = {
    cliente: values.cliente,
    nota: values.nota,
    desde: values.desde,
    ate: values.ate
  };
  if (values.limite !== undefined) {
    const limite = Number(values.limite);
    if (!Number.isInteger(limite) || limite < 1) {
      throw new Error("--limite precisa ser um inteiro positivo.");
    }
    filtros.limite = limite;
  }
  return filtros;
}

async function main(): Promise<void> {
  if (!process.env.DATABASE_URL) {
    throw new Error("DATABASE_URL não definida: a consulta precisa do banco do gateway.");
  }
  const filtros = lerFiltros(process.argv.slice(2));
  const db = await createDatabase();
  try {
    process.stdout.write(await runAuditQuery(db, filtros));
  } finally {
    await db.end();
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
