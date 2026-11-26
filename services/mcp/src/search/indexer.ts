import { normalizeCollectionChange } from "../change-events.js";
import type { ConnectGateway } from "../connect.js";
import { celString } from "./candidates.js";
import { embedUnits } from "./chunks.js";
import { EMBED_BATCH, type EmbeddingClient } from "./embeddings.js";
import { VectorIndex, type PgVectorStore, type StoredUnit, type VectorRegistry } from "./vector-store.js";

const CHANGES_PAGE = 200;
const READ_BATCH = 20;
const QUERY_PAGE = 100;
/** Deadline of one embedding request while indexing. */
export const INDEX_EMBED_TIMEOUT_MS = 60_000;

export interface IndexerTarget {
  collectionId: string;
  connectionSetId: string;
  connectionId: string;
}

export interface IndexerLog {
  info(fields: object, message: string): void;
  error(fields: object, message: string): void;
}

export interface IndexerDeps {
  gateway: ConnectGateway;
  client: EmbeddingClient;
  store: PgVectorStore;
  registry: VectorRegistry;
  target: IndexerTarget;
  log: IndexerLog;
}

export interface IndexerResult {
  indexed: number;
  deleted: number;
  full: boolean;
}

interface NoteRecord {
  path: string;
  types: string[];
  revision: string;
  frontmatter: Record<string, unknown>;
  body: string;
}

interface ChangesPage {
  events: Parameters<typeof normalizeCollectionChange>[0][];
  cursor: number | string;
  has_more: boolean;
  reset: boolean;
}

interface QueryPage {
  results?: unknown[];
  meta?: { has_more?: boolean; cursor?: string | null };
}

/** True for the notes that are embedded: Markdown files that are not templates. */
function isIndexable(note: { path: string; types: string[] }): boolean {
  return note.path.endsWith(".md") && !note.types.includes("template");
}

function folderOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

function toRecord(item: unknown): NoteRecord | null {
  const r = item as Partial<NoteRecord> | null;
  if (typeof r?.path !== "string") return null;
  return {
    path: r.path,
    types: Array.isArray(r.types) ? r.types.filter((t): t is string => typeof t === "string") : [],
    revision: typeof r.revision === "string" ? r.revision : "",
    frontmatter: r.frontmatter && typeof r.frontmatter === "object" ? r.frontmatter : {},
    body: typeof r.body === "string" ? r.body : ""
  };
}

/** What saving one note needs: its units, the vectors it can reuse and the texts to embed. */
interface NotePlan {
  note: NoteRecord;
  units: ReturnType<typeof embedUnits>;
  known: Map<string, Float32Array>;
  fresh: Array<[string, string]>;
}

/** Same hashes, lines, revision, types and folder: nothing to write. */
function sameUnits(a: StoredUnit[], b: StoredUnit[]): boolean {
  if (a.length !== b.length) return false;
  const key = (u: StoredUnit) => JSON.stringify([u.kind, u.seq, u.lineStart, u.lineEnd, u.textHash, u.revision, u.types, u.folder]);
  const left = a.map(key).sort();
  const right = b.map(key).sort();
  return left.every((k, i) => k === right[i]);
}

/**
 * Keeps the vectors of one collection in step with its notes: a full read the
 * first time (and after a gap or reset), then the `changes` feed. The stored
 * cursor moves only after everything before it is saved.
 */
export class EmbeddingIndexer {
  private running = false;

  constructor(private readonly deps: IndexerDeps) {}

  async runOnce(signal?: AbortSignal): Promise<IndexerResult> {
    if (this.running) return { indexed: 0, deleted: 0, full: false };
    this.running = true;
    try {
      const cursor = await this.deps.store.cursor();
      const result = cursor === null ? await this.fullPass(signal) : await this.incrementalPass(cursor, signal);
      if (result.indexed > 0 || result.deleted > 0) {
        this.deps.log.info({ collectionId: this.deps.target.collectionId, ...result }, "Embedding index updated");
      }
      return result;
    } finally {
      this.running = false;
    }
  }

  private index(): VectorIndex {
    const { registry, target } = this.deps;
    let index = registry.get(target.collectionId);
    if (!index) {
      index = new VectorIndex();
      registry.set(target.collectionId, index);
    }
    return index;
  }

  private async changes(after: string | undefined, signal?: AbortSignal): Promise<ChangesPage> {
    const { gateway, target } = this.deps;
    const input = after === undefined ? {} : { after: /^\d+$/.test(after) ? Number(after) : after, limit: CHANGES_PAGE };
    const page = await gateway.operation<ChangesPage>(target.connectionSetId, target.connectionId, "changes", input, { signal });
    if (!Array.isArray(page?.events)) throw new Error("Invalid changes page.");
    return page;
  }

  /** Reads the whole collection, indexes it and removes what is no longer there. */
  private async fullPass(signal?: AbortSignal): Promise<IndexerResult> {
    const { gateway, target, store } = this.deps;
    // The cursor is taken before reading, so a change made during the read is replayed next cycle.
    const start = String((await this.changes(undefined, signal)).cursor);
    const index = this.index();
    const seen = new Set<string>();
    let indexed = 0;
    let deleted = 0;
    let queryCursor: string | undefined;
    for (;;) {
      const page = await gateway.operation<QueryPage>(
        target.connectionSetId,
        target.connectionId,
        "query",
        { where: "true", include_body: true, limit: QUERY_PAGE, ...(queryCursor ? { cursor: queryCursor } : {}) },
        { signal }
      );
      const notes: NoteRecord[] = [];
      for (const item of page?.results ?? []) {
        const note = toRecord(item);
        if (!note) continue;
        if (!isIndexable(note)) continue;
        seen.add(note.path);
        notes.push(note);
      }
      indexed += await this.saveNotes(notes, signal);
      if (!page?.meta?.has_more || !page.meta.cursor) break;
      queryCursor = page.meta.cursor;
    }
    for (const path of index.paths()) {
      if (seen.has(path)) continue;
      await this.removeNote(path);
      deleted += 1;
    }
    await store.setCursor(start);
    // Only now do the vectors cover the whole collection.
    index.ready = true;
    return { indexed, deleted, full: true };
  }

  private async incrementalPass(startCursor: string, signal?: AbortSignal): Promise<IndexerResult> {
    const { store } = this.deps;
    let cursor = startCursor;
    let indexed = 0;
    let deleted = 0;
    for (;;) {
      const page = await this.changes(cursor, signal);
      const events = page.events.map(normalizeCollectionChange);
      if (page.reset || events.some((e) => e.kind === "gap")) return this.fullPass(signal);

      const pending = new Set<string>();
      for (const event of events) {
        if (event.kind === "record.created" || event.kind === "record.updated") {
          pending.add(event.path);
        } else if (event.kind === "record.deleted") {
          pending.delete(event.path);
          if (await this.removeNote(event.path)) deleted += 1;
        } else if (event.kind === "record.renamed") {
          pending.delete(event.from);
          if (event.from !== event.to) await this.moveNote(event.from, event.to);
          pending.add(event.to);
        }
      }
      const paths = [...pending];
      for (let i = 0; i < paths.length; i += READ_BATCH) {
        const batch = paths.slice(i, i + READ_BATCH);
        const notes = await this.readNotes(batch, signal);
        const present: NoteRecord[] = [];
        for (const path of batch) {
          const note = notes.get(path);
          if (note && isIndexable(note)) present.push(note);
          else if (await this.removeNote(path)) {
            // Gone, or no longer an indexable note: drop its vectors.
            deleted += 1;
          }
        }
        indexed += await this.saveNotes(present, signal);
      }

      cursor = String(page.cursor);
      await store.setCursor(cursor);
      if (!page.has_more) return { indexed, deleted, full: false };
    }
  }

  private async readNotes(paths: string[], signal?: AbortSignal): Promise<Map<string, NoteRecord>> {
    const { gateway, target } = this.deps;
    const notes = new Map<string, NoteRecord>();
    const page = await gateway.operation<QueryPage>(
      target.connectionSetId,
      target.connectionId,
      "query",
      { where: `file.path in [${paths.map(celString).join(", ")}]`, include_body: true, limit: paths.length },
      { signal }
    );
    for (const item of page?.results ?? []) {
      const note = toRecord(item);
      if (note) notes.set(note.path, note);
    }
    return notes;
  }

  /**
   * Embeds the new units of several notes together, up to EMBED_BATCH texts per request, and
   * saves each note only once all its vectors exist. Returns how many notes it wrote.
   */
  private async saveNotes(notes: NoteRecord[], signal?: AbortSignal): Promise<number> {
    let written = 0;
    let group: NotePlan[] = [];
    let pending = 0;
    const flush = async (): Promise<void> => {
      const batch = group;
      group = [];
      pending = 0;
      written += await this.embedAndSave(batch, signal);
    };
    for (const note of notes) {
      const plan = this.plan(note);
      group.push(plan);
      pending += plan.fresh.length;
      if (pending >= EMBED_BATCH) await flush();
    }
    if (group.length > 0) await flush();
    return written;
  }

  /** The units of the note, the vectors already known for them, and the texts still to embed. */
  private plan(note: NoteRecord): NotePlan {
    const known = new Map(this.index().unitsOf(note.path).map((u) => [u.textHash, u.vector]));
    const units = embedUnits(note.path, note.frontmatter, note.body);
    const fresh = [...new Map(units.filter((u) => !known.has(u.hash)).map((u) => [u.hash, u.text]))];
    return { note, units, known, fresh };
  }

  private async embedAndSave(plans: NotePlan[], signal?: AbortSignal): Promise<number> {
    const { client, store } = this.deps;
    const index = this.index();
    const texts = new Map<string, string>();
    for (const plan of plans) for (const [hash, text] of plan.fresh) texts.set(hash, text);
    if (texts.size > 0) {
      const hashes = [...texts.keys()];
      const vectors = await client.embed([...texts.values()], signal, { timeoutMs: INDEX_EMBED_TIMEOUT_MS });
      if (vectors.length !== hashes.length) throw new Error("Embedding client returned the wrong number of vectors.");
      const byHash = new Map(hashes.map((hash, i) => [hash, vectors[i]!]));
      for (const plan of plans) for (const [hash] of plan.fresh) plan.known.set(hash, byHash.get(hash)!);
    }
    let written = 0;
    for (const { note, units, known } of plans) {
      const folder = folderOf(note.path);
      const stored: StoredUnit[] = units.map((u) => ({
        path: note.path,
        kind: u.kind,
        seq: u.seq,
        lineStart: u.lineStart,
        lineEnd: u.lineEnd,
        textHash: u.hash,
        revision: note.revision,
        types: note.types,
        folder,
        vector: known.get(u.hash)!
      }));
      if (sameUnits(index.unitsOf(note.path), stored)) continue;
      await store.saveNote(note.path, stored);
      index.replaceNote(note.path, stored);
      written += 1;
    }
    return written;
  }

  /** Deletes the note's vectors if it has any. Returns whether it did. */
  private async removeNote(path: string): Promise<boolean> {
    const index = this.index();
    if (index.unitsOf(path).length === 0) return false;
    await this.deps.store.deleteNote(path);
    index.deleteNote(path);
    return true;
  }

  private async moveNote(from: string, to: string): Promise<void> {
    const index = this.index();
    if (index.unitsOf(from).length === 0) return;
    await this.deps.store.renameNote(from, to);
    index.renameNote(from, to);
  }
}
