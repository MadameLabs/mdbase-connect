import type { ConnectGateway } from "../connect.js";

export const PRE_SCORE_WEIGHTS = { title: 3, description: 2, path: 2, body: 1 } as const;
export const MAX_CANDIDATES = 30;
export const QUERY_LIMIT = 300;
export const MAX_TERMS = 8;
/** Long terms that get a body query of their own in the reserve (the longest, ties by appearance). */
export const BODY_TERMS = 8;
/** Terms the exact-name search takes, and the time each term's body query gets. */
export const MAX_EXACT_TERMS = 4;
export const EXACT_TERMS_DEADLINE_MS = 1500;
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

/** The body query for one term: nothing but this clause, so the runtime answers it
 * from its FTS5 body index (any other clause in the expression makes it a scan). */
export function bodyExpression(term: string): string {
  return `file.body.lower().contains(${celString(term)})`;
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

/** What a query result says about a note: its path and the fields the candidates show. */
interface Stored {
  path: string;
  title: string | null;
  description: string | null;
  types: string[];
  revision: string;
}

interface Row extends Stored {
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

/** The note a query result describes, or null when it has no path. */
function storedOf(item: unknown): Stored | null {
  const r = item as { path?: unknown; types?: unknown; revision?: unknown; frontmatter?: { title?: unknown; description?: unknown } | null };
  if (typeof r?.path !== "string") return null;
  return {
    path: r.path,
    title: asText(r.frontmatter?.title),
    description: asText(r.frontmatter?.description),
    types: Array.isArray(r.types) ? r.types.filter((t): t is string => typeof t === "string") : [],
    revision: typeof r.revision === "string" ? r.revision : ""
  };
}

type Filters = { types?: string[]; folder?: string };

/** Reads through the gateway: one page of a query, and a term's whole body set. Shared by
 * the candidate gatherers so each keeps the same query shape and cursor handling. */
function queryReader(gateway: ConnectGateway, connectionSetId: string, connectionId: string, filters: Filters, signal?: AbortSignal) {
  const run = async (where: string, cursor?: string) => {
    // Plain query, not `output: "metadata"`: the hosted authority refuses metadata output
    // with a body condition (query_budget_exceeded) even for a handful of matches.
    const input: Record<string, unknown> = { where, limit: QUERY_LIMIT };
    if (filters.types && filters.types.length > 0) input.types = filters.types;
    if (cursor) input.cursor = cursor;
    const page = (await gateway.operation(connectionSetId, connectionId, "query", input, { signal })) as
      { results?: unknown; meta?: { has_more?: unknown; cursor?: unknown } | null } | null;
    const more = page?.meta?.has_more === true || (typeof page?.meta?.cursor === "string" && page.meta.cursor !== "");
    const next = typeof page?.meta?.cursor === "string" && page.meta.cursor !== "" ? page.meta.cursor : null;
    return { results: Array.isArray(page?.results) ? page.results : [], more, next };
  };
  // A term's whole body set, page by page, while it stays within QUERY_LIMIT notes;
  // null when it is common (more than that, or more without a cursor to follow).
  const bodyOf = async (where: string): Promise<unknown[] | null> => {
    const all: unknown[] = [];
    let cursor: string | undefined;
    for (;;) {
      const page = await run(where, cursor);
      all.push(...page.results);
      if (!page.more) return all;
      if (all.length >= QUERY_LIMIT || page.next === null) return null;
      cursor = page.next;
    }
  };
  return { run, bodyOf };
}

/** The title/description/path expression, narrowed to a folder by prefix. */
function metaWhereOf(meta: string, folder?: string): string {
  return folder ? `(${meta}) && file.path.startsWith(${celString(folder.endsWith("/") ? folder : `${folder}/`)})` : meta;
}

/** The body expression for one term, narrowed to a folder by path range. */
function bodyWhereOf(term: string, folder?: string): string {
  return folder ? `(${bodyExpression(term)}) && ${folderRange(folder)}` : bodyExpression(term);
}

/** A term's work races a timer: `done` with its value, or late (the timer won). */
type WithinDeadline<T> = { done: true; value: T } | { done: false };

function withinDeadline<T>(work: Promise<T>, ms: number): Promise<WithinDeadline<T>> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<WithinDeadline<T>>((resolve) => {
    timer = setTimeout(() => resolve({ done: false }), ms);
  });
  const done = work.then((value): WithinDeadline<T> => ({ done: true, value }));
  return Promise.race([done, late]).finally(() => clearTimeout(timer));
}

export interface ExactPath {
  title: string | null;
  description: string | null;
  types: string[];
  revision: string;
}

/** Notes that hold the exact terms: one body query per long term (3 letters or more, through
 * the index, following its cursor, nothing when common) and one title/description/path query
 * with every term. A term whose body is not back by `deadlineMs` goes to `dropped` with its
 * partial results discarded; the other terms still return. The title query is not raced. */
export async function exactTermCandidates(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  terms: string[],
  filters: Filters,
  options: { deadlineMs: number; signal?: AbortSignal }
): Promise<{ paths: Map<string, ExactPath>; dropped: string[] }> {
  const kept = terms.slice(0, MAX_EXACT_TERMS);
  const paths = new Map<string, ExactPath>();
  if (kept.length === 0) return { paths, dropped: [] };
  const longTerms = kept.filter((t) => Array.from(t).length >= LONG_TERM_MIN);
  const shortTerms = kept.filter((t) => Array.from(t).length < LONG_TERM_MIN);
  const { run, bodyOf } = queryReader(gateway, connectionSetId, connectionId, filters, options.signal);
  const metaQuery = run(metaWhereOf(metaExpression(longTerms, shortTerms), filters.folder));
  const bodyQueries = longTerms.map((term) => withinDeadline(bodyOf(bodyWhereOf(term, filters.folder)), options.deadlineMs));
  const [metaPage, ...bodies] = await Promise.all([metaQuery, ...bodyQueries]);

  const add = (item: unknown) => {
    const stored = storedOf(item);
    if (stored && !paths.has(stored.path)) {
      paths.set(stored.path, { title: stored.title, description: stored.description, types: stored.types, revision: stored.revision });
    }
  };
  metaPage.results.forEach(add);
  const dropped: string[] = [];
  bodies.forEach((body, i) => {
    if (!body.done) dropped.push(longTerms[i]);
    else if (body.value !== null) body.value.forEach(add);
  });
  return { paths, dropped };
}

/** Candidates from one body query per long term (the BODY_TERMS longest, through the
 * FTS5 index, following its cursor) and one title/description/path query, sent
 * together. A term with more than QUERY_LIMIT body notes is common: its body is
 * dropped, since any slice of it is a path-ordered sample. */
export async function gatherCandidates(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  query: string,
  filters: Filters,
  signal?: AbortSignal
): Promise<{ candidates: Candidate[]; total: number }> {
  const { long, short } = searchTerms(query);
  if (long.length === 0 && short.length === 0) return { candidates: [], total: 0 };

  const { run, bodyOf } = queryReader(gateway, connectionSetId, connectionId, filters, signal);
  const metaWhere = metaWhereOf(metaExpression(long, short), filters.folder);
  const bodyTerms = long
    .map((term, at) => ({ term, at, len: Array.from(term).length }))
    .sort((x, y) => y.len - x.len || x.at - y.at)
    .slice(0, BODY_TERMS)
    .map((t) => t.term);
  const [metaPage, ...bodySets] = await Promise.all([run(metaWhere), ...bodyTerms.map((term) => bodyOf(bodyWhereOf(term, filters.folder)))]);

  const rows = new Map<string, Row>();
  const rowFor = (item: unknown) => {
    const stored = storedOf(item);
    if (!stored) return null;
    let row = rows.get(stored.path);
    if (!row) {
      row = { ...stored, matched: new Map() };
      rows.set(stored.path, row);
    }
    return row;
  };
  for (const item of metaPage.results) {
    const row = rowFor(item);
    if (!row) continue;
    for (const [term, whole] of [...long.map((t) => [t, false] as const), ...short.map((t) => [t, true] as const)]) {
      const fields = matchedFields(term, whole, row.path, row.title, row.description);
      if (fields.size > 0) row.matched.set(term, fields);
    }
  }
  bodySets.forEach((set, i) => {
    if (set === null) return;
    const term = bodyTerms[i];
    for (const item of set) {
      const row = rowFor(item);
      // The body counts for a term only where the term matched nothing else.
      if (row && !row.matched.has(term)) row.matched.set(term, new Set<Field>(["body"]));
    }
  });

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
