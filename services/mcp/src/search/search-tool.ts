import { GatewayOperationError, type ConnectGateway } from "../connect.js";
import { nearestFolderRule, type FolderRulesCache } from "../folder-rules.js";
import { EXACT_TERMS_DEADLINE_MS, MAX_CANDIDATES, celString, exactTermCandidates, gatherCandidates, searchTerms, type Candidate, type ExactPath } from "./candidates.js";
import type { EmbeddingClient } from "./embeddings.js";
import { bestExcerpt, excerptAt, readRecords, type Excerpt, type NoteRecord } from "./excerpt.js";
import type { IndexListCache, IndexOption } from "./indexes.js";
import { JevError, type JevClient, type JevRankOutput } from "./jev.js";
import type { NearHit, VectorIndex, VectorRegistry } from "./vector-store.js";

/** The whole response; above this, results are dropped from the end. */
export const SEARCH_RESULT_MAX_CHARS = 12_000;
/** Below this raw Jev relevance a candidate leaves the list. */
export const JEV_MIN_RELEVANCE = 0.3;
/** `start_here` needs the companion Noul at or above this. */
export const INDEX_MIN_PROBABILITY = 0.7;
/** Added to the Jev relevance of candidates under the chosen index's folder, for ordering only. */
export const JEV_INDEX_BONUS = 0.1;
export const DEFAULT_SEARCH_LIMIT = 8;
/** Linked records listed per result. */
export const MAX_RELATED = 5;
/** How long the search waits for the query embedding before using the reserve. */
export const SEMANTIC_EMBED_TIMEOUT_MS = 3_000;

export interface RelatedRecord {
  path: string;
  title: string;
  description: string | null;
}

export interface Validity {
  type: string | null;
  status: string | null;
  date: string | null;
  archived: boolean;
}

export interface SearchResult {
  path: string;
  line_start: number | null;
  line_end: number | null;
  title: string | null;
  description: string | null;
  excerpt: string | null;
  heading: string | null;
  folder_rule: string | null;
  score: number;
  related: RelatedRecord[];
  validity: Validity;
}

export interface SearchOutput {
  results: SearchResult[];
  start_here: { path: string; title: string; description: string | null } | null;
  ranking: "jev" | "pre_order";
  /** A Jev error code, "semantic_unavailable" or "terms_timeout". */
  ranking_warning?: string;
  excerpt_warning?: "bodies_unavailable";
  candidates_considered: number;
  candidates_total: number;
  truncated: boolean;
}

export interface SearchDeps {
  gateway: ConnectGateway;
  connectionSetId: string;
  jev?: JevClient | null;
  indexList?: Pick<IndexListCache, "peek">;
  folderRules?: Pick<FolderRulesCache, "peek">;
  embedding?: EmbeddingClient | null;
  vectors?: VectorRegistry;
}

export interface SearchInput {
  query: string;
  /** Exact names to find literally, besides the meaning of the query. */
  terms?: string[];
  types?: string[];
  folder?: string;
  limit?: number;
}

/** The text `search_records` sends in `content`. */
export function serializeSearch(output: SearchOutput): string {
  return JSON.stringify(output);
}

function folderOf(path: string): string {
  const at = path.lastIndexOf("/");
  return at === -1 ? "" : path.slice(0, at);
}

function fileStem(path: string): string {
  const name = path.split("/").pop() ?? path;
  return name.endsWith(".md") ? name.slice(0, -3) : name;
}

function warningOf(error: unknown): string {
  return error instanceof JevError ? error.code : "jev_unavailable";
}

const asText = (v: unknown): string | null => (typeof v === "string" ? v : null);

/** The note's types: the `types` list (or string), else `type`, else what the candidate already knows. */
function typesOf(frontmatter: Record<string, unknown>, fallback: string[]): string[] {
  const raw = frontmatter.types ?? frontmatter.type;
  const list = Array.isArray(raw) ? raw : raw === undefined ? [] : [raw];
  const types = list.filter((t): t is string => typeof t === "string");
  return types.length > 0 ? types : fallback;
}

function validityOf(path: string, frontmatter: Record<string, unknown>, fallbackTypes: string[]): Validity {
  const types = typesOf(frontmatter, fallbackTypes);
  let date: string | null = null;
  let latest = Number.NEGATIVE_INFINITY;
  for (const key of ["modified", "dateModified", "updated", "created"]) {
    const value = asText(frontmatter[key]);
    const time = value === null ? Number.NaN : Date.parse(value);
    if (!Number.isNaN(time) && time > latest) {
      latest = time;
      date = value;
    }
  }
  return {
    type: types[0] ?? null,
    status: asText(frontmatter.status),
    date,
    archived: types.includes("arquivo") || /(^|\/)99-arquivo\//.test(path)
  };
}

/** The file name a wikilink or plain related value points to: no brackets, alias, heading, folder or `.md`. */
function linkStem(raw: string): string {
  const inner = raw.replace(/^\s*\[\[/, "").replace(/\]\]\s*$/, "");
  const target = inner.split("|")[0].split("#")[0].trim();
  const name = target.split("/").pop() ?? target;
  return name.endsWith(".md") ? name.slice(0, -3) : name;
}

/** Stems of the `related` values first, then of every wikilink elsewhere in the frontmatter. */
function linkStems(frontmatter: Record<string, unknown>): string[] {
  const stems: string[] = [];
  const related = frontmatter.related;
  for (const value of Array.isArray(related) ? related : related === undefined ? [] : [related]) {
    if (typeof value === "string" && linkStem(value) !== "") stems.push(linkStem(value));
  }
  const walk = (value: unknown): void => {
    if (typeof value === "string") {
      for (const match of value.matchAll(/\[\[([^\]]+)\]\]/g)) if (linkStem(match[1]) !== "") stems.push(linkStem(match[1]));
    } else if (Array.isArray(value)) value.forEach(walk);
    else if (value !== null && typeof value === "object") Object.values(value).forEach(walk);
  };
  for (const [key, value] of Object.entries(frontmatter)) if (key !== "related") walk(value);
  return stems;
}

/** Up to MAX_RELATED distinct notes the frontmatter links to, other than the note itself. */
function relatedPaths(path: string, frontmatter: Record<string, unknown>, index: VectorIndex): string[] {
  const found: string[] = [];
  for (const stem of linkStems(frontmatter)) {
    const target = index.pathOfStem(stem);
    if (target !== null && target !== path && !found.includes(target)) found.push(target);
    if (found.length === MAX_RELATED) break;
  }
  return found;
}

/** Title and description of the linked notes, in one plain query; a failure only leaves them out. */
async function readRelated(deps: SearchDeps, connectionId: string, paths: string[], signal?: AbortSignal): Promise<Map<string, RelatedRecord>> {
  const known = new Map<string, RelatedRecord>();
  if (paths.length === 0) return known;
  try {
    const page = await deps.gateway.operation(
      deps.connectionSetId, connectionId, "query",
      { where: `file.path in [${paths.map(celString).join(", ")}]`, limit: paths.length },
      { signal }
    );
    const results = (page as { results?: unknown } | null)?.results;
    for (const item of Array.isArray(results) ? results : []) {
      const r = item as { path?: unknown; frontmatter?: { title?: unknown; description?: unknown } | null };
      if (typeof r?.path === "string") known.set(r.path, { path: r.path, title: asText(r.frontmatter?.title) ?? fileStem(r.path), description: asText(r.frontmatter?.description) });
    }
  } catch (error) {
    if (signal?.aborted) throw error;
  }
  return known;
}

interface Pool {
  candidates: Candidate[];
  total: number;
  /** The best vector of each note that came from the embedding. */
  near: Map<string, NearHit>;
  dropped: string[];
}

/** Candidates by meaning and by exact name; null when the embedding is out. */
async function semanticPool(
  deps: SearchDeps, connectionId: string, index: VectorIndex, embedding: EmbeddingClient, input: SearchInput, signal?: AbortSignal
): Promise<Pool | null> {
  const filters = { types: input.types, folder: input.folder };
  const [nearHits, exact] = await Promise.all([
    embedding.embed([input.query], signal, { timeoutMs: SEMANTIC_EMBED_TIMEOUT_MS }).then(
      ([vector]) => index.nearest(vector, MAX_CANDIDATES, filters),
      (error: unknown) => {
        if (signal?.aborted) throw error;
        return null;
      }
    ),
    exactTermCandidates(deps.gateway, deps.connectionSetId, connectionId, input.terms ?? [], filters, { deadlineMs: EXACT_TERMS_DEADLINE_MS, signal }).catch(
      (error: unknown) => {
        if (signal?.aborted) throw error;
        // The meaning search still answers; every term counts as late.
        return { paths: new Map<string, ExactPath>(), dropped: (input.terms ?? []).filter((t) => t.trim() !== "") };
      }
    )
  ]);
  if (nearHits === null) return null;
  // A note both lists found goes ahead of those only the embedding found.
  const ordered = [...nearHits.filter((h) => exact.paths.has(h.path)), ...nearHits.filter((h) => !exact.paths.has(h.path))];
  const near = new Map(nearHits.map((h) => [h.path, h]));
  const exactOnly = [...exact.paths.keys()].filter((p) => !near.has(p));
  const paths = [...ordered.map((h) => h.path), ...exactOnly].slice(0, MAX_CANDIDATES);
  const candidates = paths.map((path): Candidate => {
    const known: ExactPath | undefined = exact.paths.get(path);
    return {
      path, title: known?.title ?? null, description: known?.description ?? null, types: known?.types ?? [], revision: known?.revision ?? "",
      hits: { title: 0, description: 0, path: 0, body: 0 }, preScore: near.get(path)?.score ?? 0
    };
  });
  return { candidates, total: nearHits.length + exactOnly.length, near, dropped: exact.dropped };
}

/** Throws GatewayOperationError for gateway failures and missing query access. */
export async function searchRecords(deps: SearchDeps, connectionId: string, input: SearchInput, signal?: AbortSignal): Promise<SearchOutput> {
  const { gateway, connectionSetId } = deps;
  const connections = await gateway.listConnections(connectionSetId);
  const connection = connections.find((c) => c.id === connectionId);
  if (!connection?.operations.includes("query")) {
    throw new GatewayOperationError(
      "insufficient_collection_access",
      `${connection?.display_name ?? "This collection"} was not approved for query. Reconnect it with broader access.`
    );
  }

  const limit = input.limit ?? DEFAULT_SEARCH_LIMIT;
  const index = deps.vectors?.get(connection.collection_id) ?? null;
  let semantic = index !== null && index.ready && deps.embedding
    ? await semanticPool(deps, connectionId, index, deps.embedding, input, signal)
    : null;
  const reserve = async (): Promise<Pool> => ({
    ...(await gatherCandidates(gateway, connectionSetId, connectionId, [input.query, ...(input.terms ?? [])].join(" "), { types: input.types, folder: input.folder }, signal)),
    near: new Map(), dropped: []
  });
  let pool: Pool = semantic ?? (await reserve());
  const readAll = () => readRecords(gateway, connectionSetId, connectionId, pool.candidates.map((c) => c.path), signal);
  const output: SearchOutput = {
    results: [], start_here: null, ranking: "pre_order",
    candidates_considered: pool.candidates.length, candidates_total: pool.total, truncated: false
  };
  if (semantic === null) output.ranking_warning = "semantic_unavailable";
  if (pool.candidates.length === 0) return output;

  let records = new Map<string, NoteRecord>();
  try {
    records = await readAll();
  } catch (error) {
    if (signal?.aborted) throw error;
    if (semantic !== null) {
      // Vector candidates are only safe once this connection has read them: use the caller's own queries instead.
      semantic = null;
      output.ranking_warning = "semantic_unavailable";
      pool = await reserve();
      output.candidates_total = pool.total;
      if (pool.candidates.length === 0) return output;
      try {
        records = await readAll();
      } catch (retryError) {
        if (signal?.aborted) throw retryError;
        output.excerpt_warning = "bodies_unavailable";
      }
    } else {
      output.excerpt_warning = "bodies_unavailable";
    }
  }
  // Semantic candidates come from vectors, which may hold notes this connection cannot read.
  let candidates = semantic !== null ? pool.candidates.filter((c) => records.has(c.path)) : pool.candidates;

  // Peeks never wait: the first search of a connection may come without them.
  const indexes: IndexOption[] = deps.jev ? deps.indexList?.peek(connectionSetId, connectionId) ?? [] : [];
  const rulePaths = deps.folderRules?.peek(connectionSetId, connectionId) ?? null;
  output.candidates_considered = candidates.length;
  if (candidates.length === 0) return output;

  const { long, short } = searchTerms(input.query);
  const excerptTerms = [...(input.terms ?? []).map((t) => t.toLowerCase()), ...long, ...short];
  const excerpts = new Map<string, Excerpt | null>();
  for (const c of candidates) {
    const record = records.get(c.path);
    const hit = pool.near.get(c.path);
    if (record === undefined) excerpts.set(c.path, null);
    else if (hit?.kind === "chunk") excerpts.set(c.path, excerptAt(record.body, hit.lineStart, hit.lineEnd, excerptTerms));
    else excerpts.set(c.path, bestExcerpt(record.body, excerptTerms));
    // The query result only knows what the stored note says; titles come from the frontmatter we just read.
    const fm = record?.frontmatter ?? {};
    c.title = asText(fm.title) ?? c.title;
    c.description = asText(fm.description) ?? c.description;
  }

  let ranked: Array<{ candidate: Candidate; score: number }> = candidates.map((candidate) => ({ candidate, score: candidate.preScore }));
  if (deps.jev) {
    try {
      const jev = await deps.jev.rank({
        query: input.query,
        candidates: candidates.map((c) => ({ path: c.path, title: c.title ?? fileStem(c.path), description: c.description, excerpt: excerpts.get(c.path)?.text ?? null })),
        indexes
      }, signal);
      output.ranking = "jev";
      ranked = orderByJev(candidates, jev, indexes, output);
    } catch (error) {
      if (signal?.aborted) throw error;
      output.ranking_warning = warningOf(error);
    }
  }
  // A late term is the mildest warning: it never hides semantic_unavailable or a Jev error.
  if (semantic !== null && pool.dropped.length > 0 && output.ranking_warning === undefined) output.ranking_warning = "terms_timeout";

  const shown = ranked.slice(0, limit);
  const linked = new Map<string, string[]>();
  if (index !== null) {
    for (const { candidate } of shown) linked.set(candidate.path, relatedPaths(candidate.path, records.get(candidate.path)?.frontmatter ?? {}, index));
  }
  const relatedInfo = await readRelated(deps, connectionId, [...new Set([...linked.values()].flat())], signal);

  output.results = shown.map(({ candidate, score }) => {
    const excerpt = excerpts.get(candidate.path) ?? null;
    return {
      path: candidate.path,
      line_start: excerpt?.lineStart ?? null,
      line_end: excerpt?.lineEnd ?? null,
      title: candidate.title ?? fileStem(candidate.path),
      description: candidate.description,
      excerpt: excerpt?.text ?? null,
      heading: excerpt?.heading ?? null,
      folder_rule: rulePaths === null ? null : nearestFolderRule(candidate.path, rulePaths),
      score,
      related: (linked.get(candidate.path) ?? []).flatMap((p) => relatedInfo.get(p) ?? []),
      validity: validityOf(candidate.path, records.get(candidate.path)?.frontmatter ?? {}, candidate.types)
    };
  });
  // The cap applies to the exact text the tool sends: compact JSON of this object.
  const over = () => serializeSearch(output).length > SEARCH_RESULT_MAX_CHARS;
  if (over()) {
    if (output.results.some((r) => r.related.length > 0)) output.truncated = true;
    for (const r of output.results) r.related = [];
  }
  if (over()) {
    output.truncated = true;
    while (output.results.length > 0 && over()) output.results.pop();
  }
  return output;
}

function orderByJev(
  candidates: Candidate[],
  jev: JevRankOutput,
  indexes: IndexOption[],
  output: SearchOutput
): Array<{ candidate: Candidate; score: number }> {
  let bonusFolder: string | null = null;
  if (jev.index && jev.anyIndexFits !== null && jev.anyIndexFits >= INDEX_MIN_PROBABILITY) {
    const chosen = indexes.find((i) => i.path === jev.index?.path);
    if (chosen) {
      output.start_here = { path: chosen.path, title: chosen.title, description: chosen.description };
      const folder = folderOf(chosen.path);
      if (folder !== "") bonusFolder = `${folder}/`;
    }
  }
  return candidates
    .map((candidate) => {
      const raw = jev.relevance.get(candidate.path) ?? 0;
      const bonus = bonusFolder !== null && candidate.path.startsWith(bonusFolder) ? JEV_INDEX_BONUS : 0;
      return { candidate, score: raw, order: raw + bonus };
    })
    .filter((r) => r.score >= JEV_MIN_RELEVANCE)
    .sort((a, b) => b.order - a.order || b.candidate.preScore - a.candidate.preScore || a.candidate.path.localeCompare(b.candidate.path));
}
