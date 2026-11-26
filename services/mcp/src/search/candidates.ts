import type { ConnectGateway } from "../connect.js";

export const PRE_SCORE_WEIGHTS = { title: 3, description: 2, path: 2, body: 1 } as const;
export const MAX_CANDIDATES = 30;
export const QUERY_LIMIT = 300;
export const MAX_TERMS = 8;
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

const STOPWORDS = new Set([
  "e", "a", "o", "de", "do", "da", "em", "no", "na", "os", "as", "um", "uma", "com", "por", "para", "que",
  "dos", "das", "ao", "aos", "nos", "nas",
  "the", "and", "of", "to", "in", "for", "on", "with", "an", "is", "are", "at", "by", "from", "or"
]);

/** Lowercased, punctuation-free, de-duplicated terms; accents are kept. At most
 * MAX_TERMS survive, the longest first (ties by first appearance), long and short
 * together; each list keeps the order of appearance. `long` has 3 letters or more,
 * `short` the rest. */
export function searchTerms(query: string): { long: string[]; short: string[] } {
  const seen = new Set<string>();
  const all: Array<{ term: string; at: number; len: number }> = [];
  for (const raw of query.toLowerCase().split(/[^\p{L}\p{N}]+/u)) {
    const len = Array.from(raw).length;
    if (!raw || seen.has(raw) || STOPWORDS.has(raw) || len < 2) continue;
    seen.add(raw);
    all.push({ term: raw, at: all.length, len });
  }
  const kept = [...all].sort((a, b) => b.len - a.len || a.at - b.at).slice(0, MAX_TERMS).sort((a, b) => a.at - b.at);
  return {
    long: kept.filter((t) => t.len >= LONG_TERM_MIN).map((t) => t.term),
    short: kept.filter((t) => t.len < LONG_TERM_MIN).map((t) => t.term)
  };
}

/** A CEL string literal; the text is never concatenated raw. */
export function celString(text: string): string {
  const body = text
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t");
  return `"${body}"`;
}

const BOUNDARY_CLASS = "[^a-z0-9à-ÿ]";

/** Regex source for a short term as a whole word; terms are letters and digits only. */
function boundaryPattern(term: string): string {
  return `(^|${BOUNDARY_CLASS})${term}(${BOUNDARY_CLASS}|$)`;
}

function test(field: string, term: string, whole: boolean): string {
  const lit = celString(whole ? boundaryPattern(term) : term);
  return whole ? `${field}.lower().matches(${lit})` : `${field}.lower().contains(${lit})`;
}

/** The body query: every long term in OR, nothing else, so the runtime answers it
 * from its FTS5 body index (any other clause in the expression makes it a scan). */
export function bodyExpression(long: string[]): string {
  return long.map((t) => `file.body.lower().contains(${celString(t)})`).join(" || ");
}

/** Title, description and path for every term in one expression: long terms by
 * substring, short ones as whole words. */
export function metaExpression(long: string[], short: string[]): string {
  const one = (term: string, whole: boolean) =>
    [
      `(has(raw.title) && ${test("raw.title", term, whole)})`,
      `(has(raw.description) && ${test("raw.description", term, whole)})`,
      test("file.path", term, whole)
    ].join(" || ");
  return [...long.map((t) => one(t, false)), ...short.map((t) => one(t, true))].join(" || ");
}

/** A folder as a path range ("0" follows "/"), which the body index accepts where
 * `startsWith` would turn the body query into a scan. */
export function folderRange(folder: string): string {
  const base = folder.replace(/\/+$/, "");
  return `(file.path >= ${celString(`${base}/`)} && file.path < ${celString(`${base}0`)})`;
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

function matchedFields(term: string, whole: boolean, path: string, title: string | null, description: string | null): Set<Field> {
  const re = whole ? new RegExp(boundaryPattern(term)) : null;
  const has = (text: string) => (re ? re.test(text.toLowerCase()) : text.toLowerCase().includes(term));
  const fields = new Set<Field>();
  if (title !== null && has(title)) fields.add("title");
  if (description !== null && has(description)) fields.add("description");
  if (has(path)) fields.add("path");
  return fields;
}

const asText = (v: unknown): string | null => (typeof v === "string" ? v : null);

/** Candidates from two queries sent together: the body query (first page of the FTS5
 * index only, in path order; its cursor is ignored) and the title/description/path
 * query. A body hit is credited once, without knowing which term matched. */
export async function gatherCandidates(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  query: string,
  filters: { types?: string[]; folder?: string },
  signal?: AbortSignal
): Promise<{ candidates: Candidate[]; total: number }> {
  const { long, short } = searchTerms(query);
  if (long.length === 0 && short.length === 0) return { candidates: [], total: 0 };

  const run = async (where: string) => {
    // Plain query, not `output: "metadata"`: the hosted authority refuses metadata output
    // with a body condition (query_budget_exceeded) even for a handful of matches.
    const input: Record<string, unknown> = { where, limit: QUERY_LIMIT };
    if (filters.types && filters.types.length > 0) input.types = filters.types;
    const page = (await gateway.operation(connectionSetId, connectionId, "query", input, { signal })) as
      { results?: unknown } | null;
    return Array.isArray(page?.results) ? page.results : [];
  };
  const meta = metaExpression(long, short);
  const metaWhere = filters.folder
    ? `(${meta}) && file.path.startsWith(${celString(filters.folder.endsWith("/") ? filters.folder : `${filters.folder}/`)})`
    : meta;
  const bodyWhere = long.length === 0 ? null : filters.folder ? `(${bodyExpression(long)}) && ${folderRange(filters.folder)}` : bodyExpression(long);
  const [metaResults, bodyResults] = await Promise.all([run(metaWhere), bodyWhere === null ? Promise.resolve([]) : run(bodyWhere)]);

  const rows = new Map<string, Row & { body: boolean }>();
  const rowFor = (item: unknown) => {
    const r = item as { path?: unknown; types?: unknown; revision?: unknown; frontmatter?: { title?: unknown; description?: unknown } | null };
    if (typeof r?.path !== "string") return null;
    let row = rows.get(r.path);
    if (!row) {
      row = {
        path: r.path,
        title: asText(r.frontmatter?.title),
        description: asText(r.frontmatter?.description),
        types: Array.isArray(r.types) ? r.types.filter((t): t is string => typeof t === "string") : [],
        revision: typeof r.revision === "string" ? r.revision : "",
        matched: new Map(),
        body: false
      };
      rows.set(r.path, row);
    }
    return row;
  };
  for (const item of metaResults) {
    const row = rowFor(item);
    if (!row) continue;
    for (const [term, whole] of [...long.map((t) => [t, false] as const), ...short.map((t) => [t, true] as const)]) {
      const fields = matchedFields(term, whole, row.path, row.title, row.description);
      if (fields.size > 0) row.matched.set(term, fields);
    }
  }
  for (const item of bodyResults) {
    const row = rowFor(item);
    if (row) row.body = true;
  }
  for (const row of rows.values()) {
    // Credit the body once, unless every long term already matched outside it.
    if (row.body && !long.every((t) => row.matched.has(t))) row.matched.set("<body>", new Set<Field>(["body"]));
  }

  const candidates: Candidate[] = [];
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
    candidates.push({ path: row.path, title: row.title, description: row.description, types: row.types, revision: row.revision, hits, preScore });
  }
  const rowOf = (c: Candidate) => rows.get(c.path) as Row;
  candidates.sort(
    (a, b) => b.preScore - a.preScore || (rowOf(b).matched.size - rowOf(a).matched.size) || a.path.localeCompare(b.path)
  );
  return { candidates: candidates.slice(0, MAX_CANDIDATES), total: candidates.length };
}
