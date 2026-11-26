import { SecretBox } from "./security.js";

export interface McpRuntimeConfig {
  host: string;
  port: number;
  publicUrl: string;
  connectUrl: string;
  resource: string;
  masterKey: SecretBox;
  trustProxy: boolean;
  callLogRetentionDays: number;
  queryTextCollections: ReadonlySet<string>;
  jev: { apiKey: string; model: string; endpoint: string } | null;
  embedding: {
    apiKey: string;
    model: string;
    endpoint: string;
    dimensions: number;
    collections: Array<{ collectionId: string; connectionSetId: string; connectionId: string }>;
    intervalMs: number;
  } | null;
}

export function runtimeConfigFromEnv(env: NodeJS.ProcessEnv): McpRuntimeConfig {
  const host = env.HOST?.trim() || "127.0.0.1";
  const port = integer(env.PORT ?? "8790", "PORT", 1, 65_535);
  const publicUrl = origin(env.PUBLIC_URL ?? `http://${host}:${port}`, "PUBLIC_URL");
  const connectUrl = origin(env.MDBASE_CONNECT_URL ?? "http://127.0.0.1:8787", "MDBASE_CONNECT_URL");
  const publicHost = new URL(publicUrl).hostname;
  const connectHost = new URL(connectUrl).hostname;
  if (new URL(publicUrl).protocol !== "https:" && !isLoopback(publicHost)) {
    throw new Error("PUBLIC_URL must use HTTPS outside loopback development.");
  }
  if (new URL(connectUrl).protocol !== "https:" && !isLoopback(connectHost)) {
    throw new Error("MDBASE_CONNECT_URL must use HTTPS outside loopback development.");
  }
  const masterKey = env.MDBASE_MCP_MASTER_KEY?.trim();
  if (!masterKey) throw new Error("MDBASE_MCP_MASTER_KEY is required.");
  return {
    host,
    port,
    publicUrl,
    connectUrl,
    resource: `${publicUrl}/mcp`,
    masterKey: SecretBox.fromSecret(masterKey),
    trustProxy: env.MDBASE_MCP_TRUST_PROXY === "1",
    callLogRetentionDays: integer(env.MDBASE_MCP_CALL_LOG_RETENTION_DAYS ?? "30", "MDBASE_MCP_CALL_LOG_RETENTION_DAYS", 1, 365),
    queryTextCollections: new Set(
      (env.MDBASE_MCP_CALL_LOG_QUERY_TEXT_COLLECTIONS ?? "").split(",").map((id) => id.trim()).filter(Boolean)
    ),
    jev: jevConfig(env),
    embedding: embeddingConfig(env)
  };
}

function jevConfig(env: NodeJS.ProcessEnv): McpRuntimeConfig["jev"] {
  const apiKey = env.MDBASE_MCP_TYPESAFE_API_KEY?.trim();
  if (!apiKey) return null;
  return {
    apiKey,
    model: env.MDBASE_MCP_JEV_MODEL?.trim() || "jev-1.14.0",
    endpoint: bearerEndpoint(env.MDBASE_MCP_JEV_ENDPOINT?.trim() || "https://api.typesafe.ai/v1/systemone", "MDBASE_MCP_JEV_ENDPOINT")
  };
}

function embeddingConfig(env: NodeJS.ProcessEnv): McpRuntimeConfig["embedding"] {
  const model = env.MDBASE_MCP_EMBEDDING_MODEL?.trim();
  if (!model) return null;
  const apiKey = env.MDBASE_MCP_EMBEDDING_API_KEY?.trim() || env.MDBASE_MCP_TYPESAFE_API_KEY?.trim();
  if (!apiKey) throw new Error("MDBASE_MCP_EMBEDDING_API_KEY is required when MDBASE_MCP_EMBEDDING_MODEL is set.");
  return {
    apiKey,
    model,
    endpoint: bearerEndpoint(
      env.MDBASE_MCP_EMBEDDING_ENDPOINT?.trim() || "https://openrouter.ai/api/v1/embeddings",
      "MDBASE_MCP_EMBEDDING_ENDPOINT"
    ),
    dimensions: integer(env.MDBASE_MCP_EMBEDDING_DIMENSIONS ?? "1024", "MDBASE_MCP_EMBEDDING_DIMENSIONS", 64, 4096),
    collections: embeddingCollections(env.MDBASE_MCP_EMBEDDING_COLLECTIONS ?? ""),
    intervalMs: integer(env.MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS ?? "300", "MDBASE_MCP_EMBEDDING_INTERVAL_SECONDS", 30, 3_600) * 1000
  };
}

// Each entry is `collectionId:connectionSetId:connectionId`: three UUIDs separated by colons.
function embeddingCollections(value: string): NonNullable<McpRuntimeConfig["embedding"]>["collections"] {
  const uuid = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
  const entry = new RegExp(`^(${uuid}):(${uuid}):(${uuid})$`, "i");
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => {
      const match = entry.exec(item);
      if (!match) {
        throw new Error("MDBASE_MCP_EMBEDDING_COLLECTIONS must list collection:connection-set:connection UUID triples, separated by commas.");
      }
      return { collectionId: match[1]!, connectionSetId: match[2]!, connectionId: match[3]! };
    });
}

// The key goes to this URL as a bearer token, so only plain HTTPS URLs without embedded credentials are accepted.
function bearerEndpoint(value: string, name: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${name} must be a URL.`);
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error(`${name} must be an HTTPS URL without credentials.`);
  }
  return url.href;
}

function origin(value: string, name: string): string {
  const url = new URL(value);
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    throw new Error(`${name} must be an origin without credentials, a path, a query, or a fragment.`);
  }
  if (!['http:', 'https:'].includes(url.protocol)) throw new Error(`${name} must use HTTP or HTTPS.`);
  return url.origin;
}

function integer(value: string, name: string, minimum: number, maximum: number): number {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${name} must be an integer from ${minimum} through ${maximum}.`);
  }
  return parsed;
}

function isLoopback(hostname: string): boolean {
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "::1" || hostname === "[::1]";
}
