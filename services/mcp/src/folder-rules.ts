import type { ConnectGateway } from "./connect.js";

const RULE_NAME = "AGENTS.md";

/** The nearest AGENTS.md above the note, by whole path segments. The root
 * AGENTS.md never counts: it already arrives as the collection guide. */
export function nearestFolderRule(recordPath: string, rulePaths: readonly string[]): string | null {
  const segments = recordPath.split("/");
  const rules = new Set(rulePaths);
  // A folder AGENTS.md is itself the rule of its folder: its own rule is the one above.
  const start = segments[segments.length - 1] === RULE_NAME ? segments.length - 2 : segments.length - 1;
  for (let depth = start; depth >= 1; depth--) {
    const candidate = `${segments.slice(0, depth).join("/")}/${RULE_NAME}`;
    if (rules.has(candidate)) return candidate;
  }
  return null;
}

interface CacheEntry {
  /** null: the lookup failed and nothing older is known. */
  paths: string[] | null;
  expires: number;
}

interface Fetched {
  paths: string[] | null;
  /** false: failed, or the page was cut short. */
  complete: boolean;
}

const QUERY_LIMIT = 1000;
const FAILURE_TTL_MS = 30_000;

/** Per-process cache of the AGENTS.md paths of each connection, stale-while-
 * revalidate. Failures and truncated pages are cached for a short time so a
 * broken collection is not hammered. Folder rules are optional: nothing here
 * throws. */
export class FolderRulesCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly inflight = new Map<string, Promise<string[] | null>>();

  constructor(
    private readonly gateway: ConnectGateway,
    private readonly ttlMs = 300_000,
    private readonly now: () => number = Date.now
  ) {}

  /** Never waits: the cached list (fresh or expired) or null on a cold miss.
   * A missing or expired entry starts one background refresh. */
  peek(connectionSetId: string, connectionId: string): string[] | null {
    const key = this.keyOf(connectionSetId, connectionId);
    const hit = this.entries.get(key);
    if (!hit || hit.expires <= this.now()) void this.refresh(key, connectionSetId, connectionId);
    return hit?.paths ?? null;
  }

  async rulePaths(connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<string[]> {
    const key = this.keyOf(connectionSetId, connectionId);
    const hit = this.entries.get(key);
    if (hit && hit.expires > this.now()) return hit.paths ?? [];
    return (await this.refresh(key, connectionSetId, connectionId, signal)) ?? [];
  }

  private keyOf(connectionSetId: string, connectionId: string): string {
    return `${connectionSetId}\u0000${connectionId}`;
  }

  private refresh(key: string, connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<string[] | null> {
    const running = this.inflight.get(key);
    if (running) return running;
    const promise = this.fetch(connectionSetId, connectionId, signal)
      .then((fetched) => {
        const previous = this.entries.get(key)?.paths ?? null;
        const aborted = signal?.aborted === true;
        if (fetched.complete) this.entries.set(key, { paths: fetched.paths, expires: this.now() + this.ttlMs });
        else if (!aborted) this.entries.set(key, { paths: fetched.paths ?? previous, expires: this.now() + FAILURE_TTL_MS });
        return fetched.paths;
      })
      .finally(() => this.inflight.delete(key));
    this.inflight.set(key, promise);
    return promise;
  }

  private async fetch(connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<Fetched> {
    try {
      const connections = await this.gateway.listConnections(connectionSetId);
      const connection = connections.find((c) => c.id === connectionId);
      if (!connection?.operations.includes("query")) return { paths: [], complete: true };
      const page = await this.gateway.operation(
        connectionSetId, connectionId, "query", { where: `file.name == "${RULE_NAME}"`, limit: QUERY_LIMIT }, { signal }
      );
      const results = (page as { results?: unknown } | null)?.results;
      if (!Array.isArray(results)) return { paths: null, complete: false };
      const paths = results
        .map((r) => (r as { path?: unknown } | null)?.path)
        .filter((p): p is string => typeof p === "string");
      const meta = (page as { meta?: { has_more?: unknown; cursor?: unknown } | null }).meta;
      const more = meta?.has_more === true || (meta?.cursor !== undefined && meta.cursor !== null && meta.cursor !== "")
        || (page as { cursor?: unknown }).cursor != null;
      return { paths, complete: !more };
    } catch {
      return { paths: null, complete: false };
    }
  }
}
