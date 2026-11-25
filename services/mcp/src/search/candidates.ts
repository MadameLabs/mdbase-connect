import type { ConnectGateway } from "../connect.js";

export const PRE_SCORE_WEIGHTS = { title: 3, description: 2, path: 2, body: 1 } as const;
export const MAX_CANDIDATES = 30;
const QUERY_LIMIT = 1000;
const LONG_TERM_MIN = 3;

export interface Candidate {
  path: string;
  title: string | null;
  description: string | null;
  types: string[];
  revision: string;
  hits: { title: number; description: number; path: number; body: number };
  preScore: number;
}

/** Lowercased, punctuation-free, de-duplicated terms; accents are kept.
 * `long` has 3 letters or more, `short` the rest. */
export function searchTerms(query: string): { long: string[]; short: string[] } {
  const seen = new Set<string>();
  const long: string[] = [];
  const short: string[] = [];
  for (const raw of query.toLowerCase().split(/[^\p{L}\p{N}]+/u)) {
    if (!raw || seen.has(raw)) continue;
    seen.add(raw);
    (Array.from(raw).length >= LONG_TERM_MIN ? long : short).push(raw);
  }
  return { long, short };
}

/** A CEL string literal; the text is never concatenated raw. */
function celString(text: string): string {
  const body = text
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t");
  return `"${body}"`;
}

function termExpression(term: string, withBody: boolean): string {
  const lit = celString(term);
  const parts = [
    `(has(raw.title) && raw.title.lower().contains(${lit}))`,
    `(has(raw.description) && raw.description.lower().contains(${lit}))`,
    `file.path.lower().contains(${lit})`
  ];
  if (withBody) parts.push(`file.body.lower().contains(${lit})`);
  return parts.join(" || ");
}

type Field = keyof typeof PRE_SCORE_WEIGHTS;

interface Row {
  path: string;
  title: string | null;
  description: string | null;
  types: string[];
  revision: string;
  /** term -> fields it matched */
  matched: Map<string, Set<Field>>;
}

function matchedFields(term: string, path: string, title: string | null, description: string | null): Set<Field> {
  const fields = new Set<Field>();
  if (title?.toLowerCase().includes(term)) fields.add("title");
  if (description?.toLowerCase().includes(term)) fields.add("description");
  if (path.toLowerCase().includes(term)) fields.add("path");
  if (fields.size === 0) fields.add("body");
  return fields;
}

const asText = (v: unknown): string | null => (typeof v === "string" ? v : null);

export async function gatherCandidates(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  query: string,
  filters: { types?: string[]; folder?: string },
  signal?: AbortSignal
): Promise<{ candidates: Candidate[]; total: number }> {
  const { long, short } = searchTerms(query);
  const folderCondition = filters.folder
    ? `file.path.startsWith(${celString(filters.folder.endsWith("/") ? filters.folder : `${filters.folder}/`)})`
    : null;
  const jobs = [
    ...long.map((term) => ({ term, withBody: true })),
    ...short.map((term) => ({ term, withBody: false }))
  ];

  const pages = await Promise.all(
    jobs.map(async ({ term, withBody }) => {
      const expression = termExpression(term, withBody);
      const where = folderCondition ? `(${expression}) && ${folderCondition}` : expression;
      const input: Record<string, unknown> = { where, select: ["title", "description"], output: "metadata", limit: QUERY_LIMIT };
      if (filters.types && filters.types.length > 0) input.types = filters.types;
      const page = await gateway.operation(connectionSetId, connectionId, "query", input, { signal });
      return { term, results: (page as { results?: unknown } | null)?.results };
    })
  );

  const rows = new Map<string, Row>();
  for (const { term, results } of pages) {
    if (!Array.isArray(results)) continue;
    for (const item of results) {
      const r = item as { path?: unknown; types?: unknown; revision?: unknown; values?: { title?: unknown; description?: unknown } | null };
      if (typeof r?.path !== "string") continue;
      const title = asText(r.values?.title);
      const description = asText(r.values?.description);
      let row = rows.get(r.path);
      if (!row) {
        row = {
          path: r.path,
          title,
          description,
          types: Array.isArray(r.types) ? r.types.filter((t): t is string => typeof t === "string") : [],
          revision: typeof r.revision === "string" ? r.revision : "",
          matched: new Map()
        };
        rows.set(r.path, row);
      }
      row.matched.set(term, matchedFields(term, r.path, row.title, row.description));
    }
  }

  const candidates: Candidate[] = [];
  const distinct = new Map<string, number>();
  for (const row of rows.values()) {
    const hits = { title: 0, description: 0, path: 0, body: 0 };
    let preScore = 0;
    for (const fields of row.matched.values()) {
      let best = 0;
      for (const f of fields) {
        hits[f] += 1;
        best = Math.max(best, PRE_SCORE_WEIGHTS[f]);
      }
      preScore += best;
    }
    distinct.set(row.path, row.matched.size);
    candidates.push({ path: row.path, title: row.title, description: row.description, types: row.types, revision: row.revision, hits, preScore });
  }
  candidates.sort(
    (a, b) => b.preScore - a.preScore || (distinct.get(b.path) ?? 0) - (distinct.get(a.path) ?? 0) || a.path.localeCompare(b.path)
  );
  return { candidates: candidates.slice(0, MAX_CANDIDATES), total: candidates.length };
}
