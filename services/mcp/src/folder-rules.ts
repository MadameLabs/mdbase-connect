import type { ConnectGateway } from "./connect.js";

const RULE_NAME = "AGENTS.md";

/** The nearest AGENTS.md above the note, by whole path segments. The root
 * AGENTS.md never counts: it already arrives as the collection guide. */
export function nearestFolderRule(recordPath: string, rulePaths: readonly string[]): string | null {
  const segments = recordPath.split("/");
  const rules = new Set(rulePaths);
  for (let depth = segments.length - 1; depth >= 1; depth--) {
    const candidate = `${segments.slice(0, depth).join("/")}/${RULE_NAME}`;
    if (rules.has(candidate)) return candidate;
  }
  return null;
}

interface CacheEntry {
  paths: string[];
  expires: number;
}

/** Per-process cache of the AGENTS.md paths of each connection. Failures are
 * never cached and yield []: folder rules are optional. */
export class FolderRulesCache {
  private readonly entries = new Map<string, CacheEntry>();

  constructor(
    private readonly gateway: ConnectGateway,
    private readonly ttlMs = 300_000,
    private readonly now: () => number = Date.now
  ) {}

  async rulePaths(connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<string[]> {
    const key = `${connectionSetId}\u0000${connectionId}`;
    const hit = this.entries.get(key);
    if (hit && hit.expires > this.now()) return hit.paths;
    try {
      const connections = await this.gateway.listConnections(connectionSetId);
      const connection = connections.find((c) => c.id === connectionId);
      if (!connection?.operations.includes("query")) return [];
      const page = await this.gateway.operation(
        connectionSetId, connectionId, "query", { where: `file.name == "${RULE_NAME}"` }, { signal }
      );
      const results = (page as { results?: unknown } | null)?.results;
      if (!Array.isArray(results)) return [];
      const paths = results
        .map((r) => (r as { path?: unknown } | null)?.path)
        .filter((p): p is string => typeof p === "string");
      this.entries.set(key, { paths, expires: this.now() + this.ttlMs });
      return paths;
    } catch {
      return [];
    }
  }
}
