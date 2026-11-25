import { GatewayOperationError, type ConnectGateway } from "../connect.js";
import { nearestFolderRule, type FolderRulesCache } from "../folder-rules.js";
import { gatherCandidates, searchTerms, type Candidate } from "./candidates.js";
import { bestExcerpt, readBodies, type Excerpt } from "./excerpt.js";
import type { IndexListCache, IndexOption } from "./indexes.js";
import { JevError, type JevClient, type JevRankOutput } from "./jev.js";

/** The whole response; above this, results are dropped from the end. */
export const SEARCH_RESULT_MAX_CHARS = 12_000;
/** Below this raw Jev relevance a candidate leaves the list. */
export const JEV_MIN_RELEVANCE = 0.3;
/** `start_here` needs the companion Noul at or above this. */
export const INDEX_MIN_PROBABILITY = 0.7;
/** Added to the Jev relevance of candidates under the chosen index's folder, for ordering only. */
export const JEV_INDEX_BONUS = 0.1;
export const DEFAULT_SEARCH_LIMIT = 8;

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
}

export interface SearchOutput {
  results: SearchResult[];
  start_here: { path: string; title: string; description: string | null } | null;
  ranking: "jev" | "pre_order";
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
}

export interface SearchInput {
  query: string;
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

function warningOf(error: unknown): string {
  return error instanceof JevError ? error.code : "jev_unavailable";
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
  const { candidates, total } = await gatherCandidates(
    gateway, connectionSetId, connectionId, input.query, { types: input.types, folder: input.folder }, signal
  );
  const output: SearchOutput = {
    results: [], start_here: null, ranking: "pre_order",
    candidates_considered: candidates.length, candidates_total: total, truncated: false
  };
  if (candidates.length === 0) return output;

  // Peeks never wait: the first search of a connection may come without them.
  const indexes: IndexOption[] = deps.indexList?.peek(connectionSetId, connectionId) ?? [];
  const rulePaths = deps.folderRules?.peek(connectionSetId, connectionId) ?? null;

  let bodies = new Map<string, string>();
  try {
    bodies = await readBodies(gateway, connectionSetId, connectionId, candidates.map((c) => c.path), signal);
  } catch (error) {
    if (signal?.aborted) throw error;
    output.excerpt_warning = "bodies_unavailable";
  }
  const { long, short } = searchTerms(input.query);
  const terms = [...long, ...short];
  const excerpts = new Map<string, Excerpt | null>();
  for (const c of candidates) {
    const body = bodies.get(c.path);
    excerpts.set(c.path, body === undefined ? null : bestExcerpt(body, terms));
  }

  let ranked: Array<{ candidate: Candidate; score: number }> = candidates.map((candidate) => ({ candidate, score: candidate.preScore }));
  if (deps.jev) {
    try {
      const jev = await deps.jev.rank({
        query: input.query,
        candidates: candidates.map((c) => ({ path: c.path, title: c.title, description: c.description, excerpt: excerpts.get(c.path)?.text ?? null })),
        indexes
      }, signal);
      output.ranking = "jev";
      ranked = orderByJev(candidates, jev, indexes, output);
    } catch (error) {
      if (signal?.aborted) throw error;
      output.ranking_warning = warningOf(error);
    }
  }

  output.results = ranked.slice(0, limit).map(({ candidate, score }) => {
    const excerpt = excerpts.get(candidate.path) ?? null;
    return {
      path: candidate.path,
      line_start: excerpt?.lineStart ?? null,
      line_end: excerpt?.lineEnd ?? null,
      title: candidate.title,
      description: candidate.description,
      excerpt: excerpt?.text ?? null,
      heading: excerpt?.heading ?? null,
      folder_rule: rulePaths === null ? null : nearestFolderRule(candidate.path, rulePaths),
      score
    };
  });
  // The cap applies to the exact text the tool sends: compact JSON of this object.
  if (serializeSearch(output).length > SEARCH_RESULT_MAX_CHARS) {
    output.truncated = true;
    while (output.results.length > 0 && serializeSearch(output).length > SEARCH_RESULT_MAX_CHARS) output.results.pop();
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
