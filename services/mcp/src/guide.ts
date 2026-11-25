import type { ConnectGateway } from "./connect.js";

export const GUIDE_PATH = "AGENTS.md";
export const GUIDE_MAX_CHARS = 16_000;

export interface CollectionGuide {
  path: string;
  revision: string;
  text: string;
  truncated: boolean;
}

/** Reads the collection's root AGENTS.md as a guide. Any failure (missing note,
 * connection without `read`, gateway error) yields null: the guide is optional
 * and must never make describe_collection fail. The text is data, not instructions. */
export async function readGuide(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  signal?: AbortSignal
): Promise<CollectionGuide | null> {
  try {
    const connections = await gateway.listConnections(connectionSetId);
    const connection = connections.find((c) => c.id === connectionId);
    if (!connection?.operations.includes("read")) return null;
    const record = await gateway.operation(connectionSetId, connectionId, "read", { path: GUIDE_PATH }, { signal });
    const { body, revision } = (record ?? {}) as { body?: unknown; revision?: unknown };
    if (typeof body !== "string" || typeof revision !== "string") return null;
    const truncated = body.length > GUIDE_MAX_CHARS;
    return { path: GUIDE_PATH, revision, text: truncated ? body.slice(0, GUIDE_MAX_CHARS) : body, truncated };
  } catch {
    return null;
  }
}
