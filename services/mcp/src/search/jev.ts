import { ANY_INDEX_INSTRUCTIONS, CANDIDATE_CRITERIA, CANDIDATE_INSTRUCTIONS, PICK_INDEX_INSTRUCTIONS } from "./jev-questions.js";

const ENDPOINT = "https://api.typesafe.ai/v1/systemone";

export interface JevRankInput {
  query: string;
  candidates: Array<{ path: string; title: string | null; description: string | null; excerpt: string | null }>;
  indexes: Array<{ path: string; title: string; description: string | null }>;
}

export interface JevRankOutput {
  relevance: Map<string, number>;
  index: { path: string; probability: number } | null;
  anyIndexFits: number | null;
}

export interface JevClient {
  rank(input: JevRankInput, signal?: AbortSignal): Promise<JevRankOutput>;
}

export type JevErrorCode = "jev_unavailable" | "jev_rate_limited" | "jev_invalid_response";

export class JevError extends Error {
  constructor(public readonly code: JevErrorCode, message: string) {
    super(message);
    this.name = "JevError";
  }
}

export function createJevClient(config: { apiKey: string; model: string }, fetchImpl: typeof fetch = fetch): JevClient {
  return {
    async rank(input, signal) {
      const { body, optionPaths } = buildRequest(config.model, input);
      let response: Response;
      try {
        response = await fetchImpl(ENDPOINT, {
          method: "POST",
          headers: { "content-type": "application/json", authorization: `Bearer ${config.apiKey}` },
          body: JSON.stringify(body),
          signal
        });
      } catch (error) {
        // The message is fixed text: it never echoes the transport error, which could carry request details.
        const aborted = error instanceof Error && error.name === "AbortError";
        throw new JevError("jev_unavailable", aborted ? "Jev request aborted." : "Jev request failed.");
      }
      if (response.status === 429) throw new JevError("jev_rate_limited", "Jev rate limit reached (HTTP 429).");
      if (!response.ok) throw new JevError("jev_unavailable", `Jev responded with HTTP ${response.status}.`);
      let json: unknown;
      try {
        json = await response.json();
      } catch {
        throw new JevError("jev_invalid_response", "Jev response was not valid JSON.");
      }
      return parseResponse(json, input, optionPaths);
    }
  };
}

function buildRequest(model: string, input: JevRankInput) {
  const candidates: Record<string, unknown> = {};
  const questions: Record<string, unknown> = {};
  input.candidates.forEach((c, i) => {
    const id = `c${i + 1}`;
    candidates[id] = { path: c.path, title: c.title, description: c.description, excerpt: c.excerpt };
    questions[`rel_${id}`] = { type: "noul", instructions: CANDIDATE_INSTRUCTIONS(id), criteria: CANDIDATE_CRITERIA };
  });
  const state: Record<string, unknown> = { query: input.query, candidates };
  const optionPaths = new Map<string, string>();
  if (input.indexes.length > 0) {
    const indexes: Record<string, unknown> = {};
    const criteria: Record<string, string> = {};
    for (const index of input.indexes) {
      const name = uniqueName(optionName(index.title), optionPaths);
      optionPaths.set(name, index.path);
      indexes[name] = { path: index.path, title: index.title, description: index.description };
      criteria[name] = index.description ?? index.title;
    }
    state.indexes = indexes;
    questions.pick_index = { type: "choice", instructions: PICK_INDEX_INSTRUCTIONS, criteria };
    questions.any_index = { type: "noul", instructions: ANY_INDEX_INSTRUCTIONS };
  }
  return { body: { model, state, questions }, optionPaths };
}

function optionName(title: string): string {
  const name = title
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return name || "index";
}

function uniqueName(base: string, taken: Map<string, string>): string {
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) if (!taken.has(`${base}_${n}`)) return `${base}_${n}`;
}

function invalid(what: string): JevError {
  return new JevError("jev_invalid_response", `Jev response is missing or malformed: ${what}.`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function noul(answers: Record<string, unknown>, id: string): number {
  const answer = answers[id];
  if (!isRecord(answer) || answer.type !== "noul" || typeof answer.noul !== "number" || !Number.isFinite(answer.noul)) throw invalid(id);
  return answer.noul;
}

function parseResponse(json: unknown, input: JevRankInput, optionPaths: Map<string, string>): JevRankOutput {
  if (!isRecord(json) || !isRecord(json.answers)) throw invalid("answers");
  const answers = json.answers;
  const relevance = new Map<string, number>();
  input.candidates.forEach((c, i) => relevance.set(c.path, noul(answers, `rel_c${i + 1}`)));
  if (optionPaths.size === 0) return { relevance, index: null, anyIndexFits: null };

  const pick = answers.pick_index;
  if (!isRecord(pick) || pick.type !== "choice" || typeof pick.choice !== "string" || !isRecord(pick.probabilities)) throw invalid("pick_index");
  const path = optionPaths.get(pick.choice);
  const probability = pick.probabilities[pick.choice];
  if (path === undefined || typeof probability !== "number" || !Number.isFinite(probability)) throw invalid("pick_index");
  return { relevance, index: { path, probability }, anyIndexFits: noul(answers, "any_index") };
}
