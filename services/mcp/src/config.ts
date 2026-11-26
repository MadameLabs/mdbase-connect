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
    jev: jevConfig(env)
  };
}

function jevConfig(env: NodeJS.ProcessEnv): McpRuntimeConfig["jev"] {
  const apiKey = env.MDBASE_MCP_TYPESAFE_API_KEY?.trim();
  if (!apiKey) return null;
  return {
    apiKey,
    model: env.MDBASE_MCP_JEV_MODEL?.trim() || "jev-1.14.0",
    endpoint: jevEndpoint(env.MDBASE_MCP_JEV_ENDPOINT?.trim() || "https://api.typesafe.ai/v1/systemone")
  };
}

// The key goes to this URL as a bearer token, so only plain HTTPS URLs without embedded credentials are accepted.
function jevEndpoint(value: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error("MDBASE_MCP_JEV_ENDPOINT must be a URL.");
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error("MDBASE_MCP_JEV_ENDPOINT must be an HTTPS URL without credentials.");
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
