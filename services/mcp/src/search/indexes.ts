import type { ConnectGateway } from "../connect.js";

export const MAX_INDEX_OPTIONS = 255;

export interface IndexOption {
  path: string;
  title: string;
  description: string | null;
}

interface CacheEntry {
  /** null: the lookup failed and nothing older is known. */
  indexes: IndexOption[] | null;
  expires: number;
}

const FAILURE_TTL_MS = 30_000;

function fileStem(path: string): string {
  const name = path.split("/").pop() ?? path;
  return name.endsWith(".md") ? name.slice(0, -3) : name;
}

/** Per-process cache of each collection's index notes (type == "index"),
 * stale-while-revalidate like FolderRulesCache. Nothing here throws: failures
 * give an empty list, cached for a short time. */
export class IndexListCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly inflight = new Map<string, Promise<IndexOption[] | null>>();

  constructor(
    private readonly gateway: ConnectGateway,
    private readonly ttlMs = 300_000,
    private readonly now: () => number = Date.now
  ) {}

  /** Never waits: the cached list (fresh or expired) or null on a cold miss. */
  peek(connectionSetId: string, connectionId: string): IndexOption[] | null {
    const key = this.keyOf(connectionSetId, connectionId);
    const hit = this.entries.get(key);
    if (!hit || hit.expires <= this.now()) void this.refresh(key, connectionSetId, connectionId);
    return hit?.indexes ?? null;
  }

  async list(connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<IndexOption[]> {
    const key = this.keyOf(connectionSetId, connectionId);
    const hit = this.entries.get(key);
    if (hit && hit.expires > this.now()) return hit.indexes ?? [];
    return (await this.refresh(key, connectionSetId, connectionId, signal)) ?? [];
  }

  private keyOf(connectionSetId: string, connectionId: string): string {
    return `${connectionSetId}\u0000${connectionId}`;
  }

  private refresh(key: string, connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<IndexOption[] | null> {
    const running = this.inflight.get(key);
    if (running) return running;
    const promise = this.fetch(connectionSetId, connectionId, signal)
      .then((indexes) => {
        const aborted = signal?.aborted === true;
        if (indexes) this.entries.set(key, { indexes, expires: this.now() + this.ttlMs });
        else if (!aborted) {
          const previous = this.entries.get(key)?.indexes ?? null;
          this.entries.set(key, { indexes: previous, expires: this.now() + FAILURE_TTL_MS });
        }
        return indexes;
      })
      .finally(() => this.inflight.delete(key));
    this.inflight.set(key, promise);
    return promise;
  }

  private async fetch(connectionSetId: string, connectionId: string, signal?: AbortSignal): Promise<IndexOption[] | null> {
    try {
      const connections = await this.gateway.listConnections(connectionSetId);
      const connection = connections.find((c) => c.id === connectionId);
      if (!connection?.operations.includes("query")) return null;
      const page = await this.gateway.operation(
        connectionSetId, connectionId, "query", { where: 'type == "index"', limit: MAX_INDEX_OPTIONS }, { signal }
      );
      const results = (page as { results?: unknown } | null)?.results;
      if (!Array.isArray(results)) return null;
      const options: IndexOption[] = [];
      for (const r of results) {
        const record = r as { path?: unknown; frontmatter?: Record<string, unknown> | null } | null;
        if (typeof record?.path !== "string") continue;
        const fm = record.frontmatter ?? {};
        options.push({
          path: record.path,
          title: typeof fm.title === "string" ? fm.title : fileStem(record.path),
          description: typeof fm.description === "string" ? fm.description : null
        });
      }
      options.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
      return options.slice(0, MAX_INDEX_OPTIONS);
    } catch {
      return null;
    }
  }
}
