import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { GatewayOperationError } from "../errors.js";
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
function fakeGateway(options: { notes: Note[]; pages?: Array<Record<string, unknown>>; startCursor?: number; pageSize?: number; staleFirstPage?: number }) {
  let stale = options.staleFirstPage ?? 0;
  const pages = [...(options.pages ?? [])];
  const operation = vi.fn(async (_set: string, _conn: string, op: string, input: Record<string, unknown>) => {
    if (op === "changes") {
      if (input.after === undefined) return { events: [], cursor: options.startCursor ?? 10, has_more: false, reset: false };
      const next = pages.shift();
      if (!next) return { events: [], cursor: input.after, has_more: false, reset: false };
      return next;
    }
    // The authority refuses to resume a query by cursor: the mirror is written all the time.
    if (input.where === "true" && stale > 0) {
      stale -= 1;
      throw new GatewayOperationError("cursor_stale", "cursor expired");
    }
    if (input.cursor !== undefined) throw new GatewayOperationError("cursor_stale", "cursor expired");
    const where = String(input.where);
    const after = /^file\.path > "((?:[^"\\]|\\.)*)"$/.exec(where);
    if (where === "true" || after) {
      const size = options.pageSize ?? 100;
      const sorted = [...options.notes].sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
      const rest = after ? sorted.filter((n) => n.path > after[1]!) : sorted;
      const slice = rest.slice(0, size);
      const more = rest.length > size;
      return { results: slice.map(noteResult), meta: { has_more: more, cursor: more ? "opaque" : null } };
    }
    const wanted = [...where.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => m[1]);
    return { results: options.notes.filter((n) => wanted.includes(n.path)).map(noteResult), meta: { has_more: false, cursor: null } };
  });
  return { gateway: { operation } as unknown as ConnectGateway, operation };
}

function fakeClient() {
  const embed = vi.fn(async (texts: string[], _signal?: AbortSignal, _options?: { timeoutMs?: number }) => texts.map((t) => Float32Array.from([t.length, 1])));
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

  it("lê a coleção por caminho, nunca por cursor, e guarda o cursor do changes", async () => {
    const t = build({ notes: [{ path: "a.md" }, { path: "b.md" }, { path: "c.md" }, { path: "d.md" }, { path: "e.md" }], startCursor: 9, pageSize: 2 });
    expect(await t.indexer.runOnce()).toEqual({ indexed: 5, deleted: 0, full: true });
    const reads = t.operation.mock.calls.filter((c) => c[2] === "query").map((c) => c[3] as Record<string, unknown>);
    expect(reads.map((r) => r.where)).toEqual(["true", 'file.path > "b.md"', 'file.path > "d.md"']);
    for (const r of reads) {
      expect(r).not.toHaveProperty("cursor");
      expect(r).toMatchObject({ include_body: true, limit: 100 });
    }
    expect(t.calls[t.calls.length - 1]).toBe("cursor:9");
  });

  it("cursor_stale na primeira página: tenta a mesma página de novo e segue", async () => {
    const t = build({ notes: [{ path: "a.md" }, { path: "b.md" }], startCursor: 3, staleFirstPage: 1 });
    expect(await t.indexer.runOnce()).toEqual({ indexed: 2, deleted: 0, full: true });
    expect(t.operation.mock.calls.filter((c) => c[2] === "query" && (c[3] as { where: string }).where === "true")).toHaveLength(2);
  });

  it("cursor_stale duas vezes seguidas: o erro sobe e nada é gravado", async () => {
    const t = build({ notes: [{ path: "a.md" }], startCursor: 3, staleFirstPage: 2 });
    await expect(t.indexer.runOnce()).rejects.toMatchObject({ code: "cursor_stale" });
    expect(t.calls.filter((c) => c.startsWith("cursor:"))).toEqual([]);
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

describe("EmbeddingIndexer: correções da revisão final", () => {
  it("o índice só fica pronto depois do cursor da passada completa", async () => {
    const t = build({ notes: [{ path: "a.md" }, { path: "b.md" }], startCursor: 7 });
    const seen: boolean[] = [];
    t.embed.mockImplementation(async (texts: string[]) => {
      seen.push(t.registry.get("col-1")!.ready);
      return texts.map((x) => Float32Array.from([x.length, 1]));
    });
    await t.indexer.runOnce();
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every((ready) => !ready)).toBe(true);
    expect(t.registry.get("col-1")!.ready).toBe(true);
  });

  it("nota com linha de 5.600 caracteres grava unidades de seq único", async () => {
    const t = build({ notes: [{ path: "a.md", body: `# T\n\n${"palavra ".repeat(800)}` }] });
    await t.indexer.runOnce();
    const units = t.registry.get("col-1")!.unitsOf("a.md");
    expect(units.length).toBeGreaterThan(3);
    expect(new Set(units.map((u) => u.seq)).size).toBe(units.length);
  });

  it("junta as unidades de várias notas numa chamada só e grava cada nota depois de ter os vetores", async () => {
    const notes = Array.from({ length: 6 }, (_, i) => ({ path: `n${i}.md`, body: `texto único ${i}` }));
    const t = build({ notes, startCursor: 3 });
    const saved: number[] = [];
    t.embed.mockImplementation(async (texts: string[]) => {
      saved.push(t.store.saveNote.mock.calls.length);
      return texts.map((x) => Float32Array.from([x.length, 1]));
    });
    const result = await t.indexer.runOnce();
    expect(result.indexed).toBe(6);
    // 6 notes x (card + chunk) = 12 texts, well under one batch: a single request
    expect(t.embed).toHaveBeenCalledTimes(1);
    expect(t.embed.mock.calls[0][0]).toHaveLength(12);
    // nothing is written before the vectors exist
    expect(saved).toEqual([0]);
    expect(t.store.saveNote).toHaveBeenCalledTimes(6);
    expect(t.calls[t.calls.length - 1]).toBe("cursor:3");
  });

  it("as chamadas de embedding do indexador têm prazo de 60.000 ms", async () => {
    const t = build({ notes: [{ path: "a.md" }] });
    await t.indexer.runOnce();
    expect(t.embed.mock.calls[0][2]).toEqual({ timeoutMs: 60_000 });
  });

  it("falha no lote não grava nenhuma nota dele nem avança o cursor", async () => {
    const t = build({ notes: [{ path: "a.md" }, { path: "b.md" }] });
    t.embed.mockRejectedValueOnce(new EmbeddingError("embedding_unavailable", "fora do ar"));
    await expect(t.indexer.runOnce()).rejects.toThrow("fora do ar");
    expect(t.store.saveNote).not.toHaveBeenCalled();
    expect(t.store.setCursor).not.toHaveBeenCalled();
    expect(t.registry.get("col-1")?.ready ?? false).toBe(false);
  });
});
