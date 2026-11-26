// Texts per request. Longer lists are split into consecutive batches of this size.
export const EMBED_BATCH = 64;

export interface EmbeddingClient {
  model: string;
  // Unit-length vectors, in the same order as `texts`.
  // `timeoutMs` bounds each HTTP request; a timeout is an embedding_unavailable error.
  embed(texts: string[], signal?: AbortSignal, options?: { timeoutMs?: number }): Promise<Float32Array[]>;
}

export type EmbeddingErrorCode = "embedding_unavailable" | "embedding_rate_limited" | "embedding_invalid_response";

export class EmbeddingError extends Error {
  constructor(public readonly code: EmbeddingErrorCode, message: string) {
    super(message);
    this.name = "EmbeddingError";
  }
}

export function createEmbeddingClient(
  config: { apiKey: string; model: string; endpoint: string; dimensions: number },
  fetchImpl: typeof fetch = fetch
): EmbeddingClient {
  return {
    model: config.model,
    async embed(texts, signal, options) {
      const vectors: Float32Array[] = [];
      for (let start = 0; start < texts.length; start += EMBED_BATCH) {
        vectors.push(...(await embedBatch(config, fetchImpl, texts.slice(start, start + EMBED_BATCH), signal, options?.timeoutMs)));
      }
      return vectors;
    }
  };
}

async function embedBatch(
  config: { apiKey: string; model: string; endpoint: string; dimensions: number },
  fetchImpl: typeof fetch,
  texts: string[],
  signal?: AbortSignal,
  timeoutMs?: number
): Promise<Float32Array[]> {
  if (texts.length === 0) return [];
  const signals = [signal, timeoutMs === undefined ? undefined : AbortSignal.timeout(timeoutMs)].filter((s): s is AbortSignal => s !== undefined);
  const combined = signals.length > 1 ? AbortSignal.any(signals) : signals[0];
  const body = { model: config.model, input: texts, dimensions: config.dimensions };
  let response: Response;
  try {
    response = await fetchImpl(config.endpoint, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${config.apiKey}` },
      body: JSON.stringify(body),
      signal: combined
    });
  } catch (error) {
    // The message is fixed text: it never echoes the transport error, which could carry request details.
    const name = error instanceof Error ? error.name : "";
    const message = name === "TimeoutError" ? "Embedding request timed out." : name === "AbortError" ? "Embedding request aborted." : "Embedding request failed.";
    throw new EmbeddingError("embedding_unavailable", message);
  }
  if (response.status === 429) throw new EmbeddingError("embedding_rate_limited", "Embedding rate limit reached (HTTP 429).");
  if (!response.ok) throw new EmbeddingError("embedding_unavailable", `Embedding service responded with HTTP ${response.status}.`);
  let json: unknown;
  try {
    json = await response.json();
  } catch {
    throw new EmbeddingError("embedding_invalid_response", "Embedding response was not valid JSON.");
  }
  return parseResponse(json, texts.length, config.dimensions);
}

function invalid(what: string): EmbeddingError {
  return new EmbeddingError("embedding_invalid_response", `Embedding response is missing or malformed: ${what}.`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Each item carries its position in `index`, which may not match the order of the array.
function parseResponse(json: unknown, count: number, dimensions: number): Float32Array[] {
  if (!isRecord(json) || !Array.isArray(json.data) || json.data.length !== count) throw invalid("data");
  const vectors: Array<Float32Array | undefined> = new Array(count);
  for (const item of json.data) {
    if (!isRecord(item)) throw invalid("data");
    const index = item.index;
    if (typeof index !== "number" || !Number.isInteger(index) || index < 0 || index >= count || vectors[index] !== undefined) {
      throw invalid("index");
    }
    vectors[index] = unitVector(item.embedding, dimensions);
  }
  return vectors as Float32Array[];
}

function unitVector(embedding: unknown, dimensions: number): Float32Array {
  if (!Array.isArray(embedding) || embedding.length !== dimensions) throw invalid("embedding");
  const values: number[] = [];
  for (const value of embedding) {
    if (typeof value !== "number" || !Number.isFinite(value)) throw invalid("embedding");
    values.push(value);
  }
  const norm = Math.hypot(...values);
  if (norm === 0) throw invalid("embedding");
  return Float32Array.from(values, (value) => value / norm);
}
