import type { DatabaseConnection, DatabasePool } from "../db.js";

/** One stored vector of a note: a card or a body chunk, without its text. */
export interface StoredUnit {
  path: string;
  kind: "card" | "chunk";
  lineStart: number;
  lineEnd: number;
  textHash: string;
  revision: string;
  types: string[];
  /** Full parent folder of the note: "13-engenharia/01-planejamento", or "" at the root. */
  folder: string;
  /** Unit-length vector; the dot product with a query is the cosine similarity. */
  vector: Float32Array;
}

export interface NearHit {
  path: string;
  score: number;
  lineStart: number;
  lineEnd: number;
  kind: "card" | "chunk";
}

export interface NearFilter {
  types?: string[];
  /** Notes whose path starts with `folder + "/"`. An empty or missing folder does not filter. */
  folder?: string;
}

/** The parent folder of a note path, or "" for a note at the root. */
function folderOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

/** The file name without `.md`. */
function stemOf(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return name.endsWith(".md") ? name.slice(0, -3) : name;
}

function depthOf(path: string): number {
  return path.split("/").length;
}

/**
 * In-memory vectors of one collection and model. Comparison is brute force: the
 * collection holds tens of thousands of vectors, and nearest() is a plain dot
 * product over them. Vectors stay as Float32Array, never converted to number[].
 */
export class VectorIndex {
  private readonly notes = new Map<string, StoredUnit[]>();
  private readonly stems = new Map<string, Set<string>>();
  private vectorCount = 0;

  /** Replaces every vector of the note. An empty list removes the note. */
  replaceNote(path: string, units: StoredUnit[]): void {
    this.removeNote(path);
    if (units.length === 0) return;
    this.notes.set(path, [...units]);
    this.vectorCount += units.length;
    const stem = stemOf(path);
    const paths = this.stems.get(stem) ?? new Set<string>();
    paths.add(path);
    this.stems.set(stem, paths);
  }

  deleteNote(path: string): void {
    this.removeNote(path);
  }

  /** Moves the note and rewrites the path and folder of its vectors. Replaces whatever was at `to`. */
  renameNote(from: string, to: string): void {
    const units = this.notes.get(from);
    if (!units) return;
    this.removeNote(from);
    this.removeNote(to);
    const folder = folderOf(to);
    this.replaceNote(
      to,
      units.map((u) => ({ ...u, path: to, folder }))
    );
  }

  /** Text hashes of the note's vectors, for the indexer to skip what did not change. */
  hashes(path: string): Set<string> {
    return new Set((this.notes.get(path) ?? []).map((u) => u.textHash));
  }

  /** Number of stored vectors across all notes. */
  size(): number {
    return this.vectorCount;
  }

  /**
   * The note whose file name, without `.md`, is `stem`. With several notes of that
   * name, the one with the shallowest path wins, ties broken alphabetically.
   */
  pathOfStem(stem: string): string | null {
    const paths = this.stems.get(stem);
    if (!paths || paths.size === 0) return null;
    return [...paths].sort((a, b) => depthOf(a) - depthOf(b) || (a < b ? -1 : a > b ? 1 : 0))[0];
  }

  /**
   * Scores each note by its best vector and returns up to `k` notes, best first.
   * The filter applies before the cut, so `k` slots are never spent on notes that
   * would be dropped.
   */
  nearest(query: Float32Array, k: number, filter: NearFilter = {}): NearHit[] {
    const prefix = filter.folder ? `${filter.folder}/` : null;
    const types = filter.types && filter.types.length > 0 ? new Set(filter.types) : null;
    const hits: NearHit[] = [];

    for (const [path, units] of this.notes) {
      if (prefix !== null && !path.startsWith(prefix)) continue;
      let best: NearHit | null = null;
      for (const unit of units) {
        if (types !== null && !unit.types.some((t) => types.has(t))) continue;
        const score = dot(query, unit.vector);
        if (best === null || score > best.score) {
          best = { path, score, lineStart: unit.lineStart, lineEnd: unit.lineEnd, kind: unit.kind };
        }
      }
      if (best !== null) hits.push(best);
    }

    hits.sort((a, b) => b.score - a.score);
    return hits.slice(0, Math.max(0, k));
  }

  private removeNote(path: string): void {
    const units = this.notes.get(path);
    if (!units) return;
    this.notes.delete(path);
    this.vectorCount -= units.length;
    const stem = stemOf(path);
    const paths = this.stems.get(stem);
    if (paths) {
      paths.delete(path);
      if (paths.size === 0) this.stems.delete(stem);
    }
  }
}

function dot(a: Float32Array, b: Float32Array): number {
  const length = Math.min(a.length, b.length);
  let sum = 0;
  for (let i = 0; i < length; i++) sum += a[i] * b[i];
  return sum;
}

/** Shares the index of each collection between the indexer and the search. */
export class VectorRegistry {
  private readonly indexes = new Map<string, VectorIndex>();

  get(collectionId: string): VectorIndex | null {
    return this.indexes.get(collectionId) ?? null;
  }

  set(collectionId: string, index: VectorIndex): void {
    this.indexes.set(collectionId, index);
  }
}

interface VectorRow {
  path: string;
  kind: "card" | "chunk";
  line_start: number;
  line_end: number;
  text_hash: string;
  revision: string;
  types: string[];
  folder: string;
  vector: Buffer | Uint8Array;
}

/**
 * Postgres copy of the vectors of one collection and model. The rows never hold
 * the text of a unit, only its hash. Vectors are stored as little-endian float32
 * bytes, the layout Float32Array has on the little-endian hosts this runs on.
 */
export class PgVectorStore {
  constructor(
    private readonly db: DatabasePool,
    private readonly collectionId: string,
    private readonly model: string
  ) {}

  async load(): Promise<VectorIndex> {
    const { rows } = await this.db.query<VectorRow>(
      `SELECT path, kind, line_start, line_end, text_hash, revision, types, folder, vector
       FROM mcp_embedding_vectors WHERE collection_id = $1 AND model = $2
       ORDER BY path, kind, line_start`,
      [this.collectionId, this.model]
    );
    const byPath = new Map<string, StoredUnit[]>();
    for (const row of rows) {
      const units = byPath.get(row.path) ?? [];
      units.push({
        path: row.path,
        kind: row.kind,
        lineStart: row.line_start,
        lineEnd: row.line_end,
        textHash: row.text_hash,
        revision: row.revision,
        types: row.types,
        folder: row.folder,
        vector: bytesToFloat32(row.vector)
      });
      byPath.set(row.path, units);
    }
    const index = new VectorIndex();
    for (const [path, units] of byPath) index.replaceNote(path, units);
    return index;
  }

  /** Replaces the note's rows in one transaction. */
  async saveNote(path: string, units: StoredUnit[]): Promise<void> {
    await this.inTransaction(async (client) => {
      await client.query(
        "DELETE FROM mcp_embedding_vectors WHERE collection_id = $1 AND model = $2 AND path = $3",
        [this.collectionId, this.model, path]
      );
      if (units.length === 0) return;
      const values: unknown[] = [];
      const tuples = units.map((u, i) => {
        const base = i * 11;
        values.push(
          this.collectionId,
          this.model,
          u.path,
          u.kind,
          u.lineStart,
          u.lineEnd,
          u.textHash,
          u.revision,
          JSON.stringify(u.types),
          u.folder,
          float32ToBytes(u.vector)
        );
        return `($${base + 1}::uuid, $${base + 2}::text, $${base + 3}::text, $${base + 4}::text, $${base + 5}::integer, $${base + 6}::integer, $${base + 7}::text, $${base + 8}::text, $${base + 9}::jsonb, $${base + 10}::text, $${base + 11}::bytea)`;
      });
      await client.query(
        `INSERT INTO mcp_embedding_vectors (collection_id, model, path, kind, line_start, line_end, text_hash, revision, types, folder, vector)
         VALUES ${tuples.join(", ")}`,
        values
      );
    });
  }

  async deleteNote(path: string): Promise<void> {
    await this.db.query(
      "DELETE FROM mcp_embedding_vectors WHERE collection_id = $1 AND model = $2 AND path = $3",
      [this.collectionId, this.model, path]
    );
  }

  /** Moves the note's rows to `to`, replacing any rows already there, in one transaction. */
  async renameNote(from: string, to: string): Promise<void> {
    const folder = folderOf(to);
    await this.inTransaction(async (client) => {
      await client.query(
        "DELETE FROM mcp_embedding_vectors WHERE collection_id = $1 AND model = $2 AND path = $3",
        [this.collectionId, this.model, to]
      );
      await client.query(
        "UPDATE mcp_embedding_vectors SET path = $4, folder = $5 WHERE collection_id = $1 AND model = $2 AND path = $3",
        [this.collectionId, this.model, from, to, folder]
      );
    });
  }

  async cursor(): Promise<string | null> {
    const { rows } = await this.db.query<{ cursor: string | null }>(
      "SELECT cursor FROM mcp_embedding_state WHERE collection_id = $1 AND model = $2",
      [this.collectionId, this.model]
    );
    return rows[0]?.cursor ?? null;
  }

  async setCursor(cursor: string | null): Promise<void> {
    await this.db.query(
      `INSERT INTO mcp_embedding_state (collection_id, model, cursor, indexed_at)
       VALUES ($1::uuid, $2::text, $3::text, now())
       ON CONFLICT (collection_id, model) DO UPDATE SET cursor = EXCLUDED.cursor, indexed_at = EXCLUDED.indexed_at`,
      [this.collectionId, this.model, cursor]
    );
  }

  /** Runs `work` on one connection between BEGIN and COMMIT, rolling back if it throws. */
  private async inTransaction(work: (client: DatabaseConnection) => Promise<void>): Promise<void> {
    const client = await this.db.connect();
    try {
      await client.query("BEGIN");
      await work(client);
      await client.query("COMMIT");
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }
}

function float32ToBytes(vector: Float32Array): Buffer {
  return Buffer.from(vector.buffer, vector.byteOffset, vector.byteLength);
}

function bytesToFloat32(bytes: Buffer | Uint8Array): Float32Array {
  // Copy into a fresh, aligned buffer: pool buffers may start at an odd offset.
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return new Float32Array(copy.buffer);
}
