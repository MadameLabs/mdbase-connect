import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import type { EmbeddingClient } from "./embeddings.js";
import { EmbeddingError } from "./embeddings.js";
import { EmbeddingIndexer } from "./indexer.js";
import { VectorIndex, VectorRegistry, type PgVectorStore } from "./vector-store.js";

const TARGET = { collectionId: "col-1", connectionSetId: "set-1", connectionId: "conn-1" };

interface Note {
  path: string;
  types?: string[];
  revision?: string;
  frontmatter?: Record<string, unknown>;
  body?: string;
}

function noteResult(n: Note) {
  return { path: n.path, types: n.types ?? ["note"], revision: n.revision ?? "r1", frontmatter: n.frontmatter ?? {}, body: n.body ?? "texto da nota" };
}

/** Scripted gateway: `query` answers from `notes` (full read pages of `pageSize`, or by file.path), `changes` from `pages`. */
function fakeGateway(options: { notes: Note[]; pages?: Array<Record<string, unknown>>; startCursor?: number; pageSize?: number }) {
  const pages = [...(options.pages ?? [])];
  const operation = vi.fn(async (_set: string, _conn: string, op: string, input: Record<string, unknown>) => {
    if (op === "changes") {
      if (input.after === undefined) return { events: [], cursor: options.startCursor ?? 10, has_more: false, reset: false };
      const next = pages.shift();
      if (!next) return { events: [], cursor: input.after, has_more: false, reset: false };
      return next;
    }
    const where = String(input.where);
    if (where === "true") {
      const size = options.pageSize ?? 100;
      const from = input.cursor ? Number(input.cursor) : 0;
      const slice = options.notes.slice(from, from + size);
      const more = from + size < options.notes.length;
      return { results: slice.map(noteResult), meta: { has_more: more, cursor: more ? String(from + size) : null } };
    }
    const wanted = [...where.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => m[1]);
    return { results: options.notes.filter((n) => wanted.includes(n.path)).map(noteResult), meta: { has_more: false, cursor: null } };
  });
  return { gateway: { operation } as unknown as ConnectGateway, operation };
}

function fakeClient() {
  const embed = vi.fn(async (texts: string[]) => texts.map((t) => Float32Array.from([t.length, 1])));
  return { client: { model: "m", embed } as EmbeddingClient, embed };
}

function fakeStore(initial: string | null = null) {
  const calls: string[] = [];
  let cursor = initial;
  const store = {
    load: vi.fn(),
    saveNote: vi.fn(async (path: string) => void calls.push(`save:${path}`)),
    deleteNote: vi.fn(async (path: string) => void calls.push(`delete:${path}`)),
    renameNote: vi.fn(async (from: string, to: string) => void calls.push(`rename:${from}>${to}`)),
    cursor: vi.fn(async () => cursor),
    setCursor: vi.fn(async (value: string | null) => {
      cursor = value;
      calls.push(`cursor:${value}`);
    })
  };
  return { store: store as unknown as PgVectorStore & typeof store, calls };
}

function build(options: Parameters<typeof fakeGateway>[0], cursor: string | null = null, existing?: VectorIndex) {
  const g = fakeGateway(options);
  const c = fakeClient();
  const s = fakeStore(cursor);
  const registry = new VectorRegistry();
  if (existing) registry.set(TARGET.collectionId, existing);
  const log = { info: vi.fn(), error: vi.fn() };
  const indexer = new EmbeddingIndexer({ gateway: g.gateway, client: c.client, store: s.store, registry, target: TARGET, log });
  return { indexer, registry, log, ...g, ...c, ...s };
}

const event = (type: string, cursor: number, payload: Record<string, unknown>) => ({ type, cursor, occurred_at: "2026-01-01T00:00:00Z", payload });
const created = (cursor: number, path: string) => event("mdbase.record.created", cursor, { path });
const updated = (cursor: number, path: string) => event("mdbase.record.modified", cursor, { path });
const deleted = (cursor: number, path: string) => event("mdbase.record.deleted", cursor, { path });
const renamed = (cursor: number, from: string, to: string) => event("mdbase.record.renamed", cursor, { from, to });
const page = (events: unknown[], cursor: number, extra: Record<string, unknown> = {}) => ({ events, cursor, has_more: false, reset: false, ...extra });

describe("EmbeddingIndexer", () => {
  it("primeira vez indexa a coleção inteira e guarda o cursor de antes da leitura", async () => {
    const t = build({ notes: [{ path: "a.md" }, { path: "pasta/b.md" }, { path: "c.md" }], startCursor: 42, pageSize: 2 });
    const result = await t.indexer.runOnce();
    expect(result).toEqual({ indexed: 3, deleted: 0, full: true });
    expect(t.calls.filter((c) => c.startsWith("save:")).sort()).toEqual(["save:a.md", "save:c.md", "save:pasta/b.md"]);
    // The cursor is written last, with the value read before the first query.
    expect(t.calls[t.calls.length - 1]).toBe("cursor:42");
    const firstQuery = t.operation.mock.calls.findIndex((c) => c[2] === "query");
    expect(t.operation.mock.calls[0][2]).toBe("changes");
    expect(t.operation.mock.calls[0][3]).not.toHaveProperty("after");
    expect(firstQuery).toBeGreaterThan(0);
    expect(t.operation.mock.calls[firstQuery][3]).toMatchObject({ where: "true", include_body: true });
    const index = t.registry.get("col-1")!;
    expect(index.paths().sort()).toEqual(["a.md", "c.md", "pasta/b.md"]);
    expect(index.unitsOf("pasta/b.md")[0]).toMatchObject({ folder: "pasta", types: ["note"], revision: "r1" });
  });

  it("só embeda unidade de hash novo", async () => {
    const body = "texto estável";
    const first = build({ notes: [{ path: "a.md", body }], startCursor: 5 });
    await first.indexer.runOnce();
    const index = first.registry.get("col-1")!;
    const before = index.unitsOf("a.md");
    expect(first.embed).toHaveBeenCalledTimes(1);

    // Same note, new revision and a new chunk of text: only the new text is embedded.
    const second = build(
      { notes: [{ path: "a.md", body: `${body}\n\n# Novo\ntrecho novo`, revision: "r2" }], pages: [page([updated(11, "a.md")], 11)] },
      "5",
      index
    );
    const result = await second.indexer.runOnce();
    expect(result).toEqual({ indexed: 1, deleted: 0, full: false });
    const embedded = second.embed.mock.calls.flatMap((c) => c[0] as string[]);
    expect(embedded.length).toBeGreaterThan(0);
    expect(embedded).not.toContain(body);
    const cardBefore = before.find((u) => u.kind === "card")!;
    const after = index.unitsOf("a.md");
    expect(after.every((u) => u.revision === "r2")).toBe(true);
    // Reused vector for a unit whose hash did not change.
    const sameHash = after.find((u) => u.textHash === cardBefore.textHash);
    if (sameHash) expect(sameHash.vector).toBe(cardBefore.vector);
    expect(second.store.setCursor).toHaveBeenLastCalledWith("11");
  });

  it("indexador apaga vetores de nota apagada e move os de renomeada", async () => {
    const seed = build({ notes: [{ path: "velha.md" }, { path: "morta.md" }], startCursor: 1 });
    await seed.indexer.runOnce();
    const index = seed.registry.get("col-1")!;
    const t = build(
      {
        notes: [{ path: "nova/pasta.md", body: "texto novo depois de renomear" }],
        pages: [page([deleted(2, "morta.md"), renamed(3, "velha.md", "nova/pasta.md"), renamed(4, "mesma.md", "mesma.md")], 4)]
      },
      "1",
      index
    );
    const result = await t.indexer.runOnce();
    expect(result.deleted).toBe(1);
    expect(t.calls).toContain("delete:morta.md");
    expect(t.calls).toContain("rename:velha.md>nova/pasta.md");
    expect(t.calls).not.toContain("rename:mesma.md>mesma.md");
    expect(index.paths().sort()).toEqual(["nova/pasta.md"]);
    expect(index.unitsOf("nova/pasta.md")[0].folder).toBe("nova");
    expect(t.calls[t.calls.length - 1]).toBe("cursor:4");
  });

  it("reset ou gap refaz a coleção inteira", async () => {
    for (const trigger of [page([], 99, { reset: true }), page([event("mdbase.collection.invalidated", 20, {})], 20)]) {
      const seed = build({ notes: [{ path: "some.md" }, { path: "fica.md" }], startCursor: 1 });
      await seed.indexer.runOnce();
      const index = seed.registry.get("col-1")!;
      const t = build({ notes: [{ path: "fica.md" }], pages: [trigger], startCursor: 77 }, "1", index);
      const result = await t.indexer.runOnce();
      expect(result.full).toBe(true);
      expect(index.paths()).toEqual(["fica.md"]);
      expect(t.calls).toContain("delete:some.md");
      expect(t.calls[t.calls.length - 1]).toBe("cursor:77");
    }
  });

  it("falha do embedding não avança o cursor", async () => {
    const t = build({ notes: [{ path: "a.md" }] });
    t.embed.mockRejectedValueOnce(new EmbeddingError("embedding_unavailable", "fora do ar"));
    await expect(t.indexer.runOnce()).rejects.toThrow("fora do ar");
    expect(t.store.setCursor).not.toHaveBeenCalled();

    const inc = build({ notes: [{ path: "a.md" }], pages: [page([created(11, "a.md")], 11), page([created(11, "a.md")], 11)] }, "10");
    inc.embed.mockRejectedValueOnce(new EmbeddingError("embedding_unavailable", "fora do ar"));
    await expect(inc.indexer.runOnce()).rejects.toThrow("fora do ar");
    expect(inc.store.setCursor).not.toHaveBeenCalled();
    // The next cycle retries from the same cursor and succeeds.
    await inc.indexer.runOnce();
    expect(inc.store.setCursor).toHaveBeenCalledWith("11");
  });

  it("ciclo não se sobrepõe", async () => {
    const t = build({ notes: [{ path: "a.md" }] });
    let release!: () => void;
    t.embed.mockImplementationOnce(
      (texts: string[]) => new Promise((resolve) => { release = () => resolve(texts.map(() => Float32Array.from([1, 1]))); })
    );
    const first = t.indexer.runOnce();
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    expect(await t.indexer.runOnce()).toEqual({ indexed: 0, deleted: 0, full: false });
    release();
    expect((await first).indexed).toBe(1);
    // Free again after the cycle ends.
    expect((await t.indexer.runOnce()).full).toBe(false);
  });

  it("template e arquivo que não é Markdown ficam de fora", async () => {
    const t = build({
      notes: [{ path: "a.md" }, { path: "modelo.md", types: ["template"] }, { path: "dados.json" }, { path: "misto.md", types: ["note", "template"] }]
    });
    const result = await t.indexer.runOnce();
    expect(result.indexed).toBe(1);
    expect(t.registry.get("col-1")!.paths()).toEqual(["a.md"]);
  });
});
