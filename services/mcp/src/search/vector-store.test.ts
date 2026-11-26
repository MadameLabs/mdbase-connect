import { describe, expect, it } from "vitest";
import { createDatabase } from "../db.js";
import { PgVectorStore, VectorIndex, type StoredUnit } from "./vector-store.js";

const COLLECTION = "11111111-1111-4111-8111-111111111111";
const MODEL = "modelo-teste";

function vec(...values: number[]): Float32Array {
  return new Float32Array(values);
}

function unit(path: string, overrides: Partial<StoredUnit> = {}): StoredUnit {
  const folder = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  return {
    path,
    kind: "chunk",
    seq: overrides.kind === "card" ? 0 : (overrides.lineStart ?? 1),
    lineStart: 1,
    lineEnd: 5,
    textHash: `hash-${path}-${overrides.lineStart ?? 1}`,
    revision: "rev-1",
    types: ["nota"],
    folder,
    vector: vec(1, 0),
    ...overrides
  };
}

describe("VectorIndex", () => {
  it("nearest pontua a nota pelo melhor vetor e devolve o trecho vencedor", () => {
    const index = new VectorIndex();
    index.replaceNote("a/alfa.md", [
      unit("a/alfa.md", { kind: "card", lineStart: 0, lineEnd: 0, vector: vec(1, 0) }),
      unit("a/alfa.md", { kind: "chunk", lineStart: 10, lineEnd: 20, vector: vec(0, 1) })
    ]);
    index.replaceNote("a/beta.md", [unit("a/beta.md", { vector: vec(Math.SQRT1_2, Math.SQRT1_2) })]);

    const result = index.nearest(vec(0, 1), 5);

    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({ path: "a/alfa.md", score: 1, lineStart: 10, lineEnd: 20, kind: "chunk" });
    expect(result[1].path).toBe("a/beta.md");
    expect(result[1].score).toBeCloseTo(Math.SQRT1_2, 6);
  });

  it("nearest filtra por pasta e por tipo antes do corte", () => {
    const index = new VectorIndex();
    index.replaceNote("13-engenharia/01-planejamento/plano.md", [
      unit("13-engenharia/01-planejamento/plano.md", { types: ["task"], vector: vec(0.9, 0.1) })
    ]);
    index.replaceNote("13-engenharia/02-sistemas/sistema.md", [
      unit("13-engenharia/02-sistemas/sistema.md", { types: ["projeto"], vector: vec(1, 0) })
    ]);
    index.replaceNote("outra/tarefa.md", [unit("outra/tarefa.md", { types: ["task"], vector: vec(0.5, 0.5) })]);

    // Com k = 1, o corte só vem depois do filtro: a nota de maior pontuação
    // fora da pasta não pode tomar a vaga da que está dentro.
    expect(index.nearest(vec(1, 0), 1, { folder: "13-engenharia" }).map((r) => r.path)).toEqual([
      "13-engenharia/02-sistemas/sistema.md"
    ]);
    expect(index.nearest(vec(1, 0), 1, { folder: "13-engenharia/01-planejamento" }).map((r) => r.path)).toEqual([
      "13-engenharia/01-planejamento/plano.md"
    ]);
    expect(index.nearest(vec(1, 0), 5, { types: ["task"] }).map((r) => r.path)).toEqual([
      "13-engenharia/01-planejamento/plano.md",
      "outra/tarefa.md"
    ]);
    expect(index.nearest(vec(1, 0), 5, { folder: "13-engenharia", types: ["task"] }).map((r) => r.path)).toEqual([
      "13-engenharia/01-planejamento/plano.md"
    ]);
  });

  it("nearest com folder de barra final acha as notas da pasta", () => {
    const index = new VectorIndex();
    index.replaceNote("13-engenharia/a.md", [unit("13-engenharia/a.md", { vector: vec(1, 0) })]);
    index.replaceNote("outra/b.md", [unit("outra/b.md", { vector: vec(1, 0) })]);
    expect(index.nearest(vec(1, 0), 5, { folder: "13-engenharia/" }).map((r) => r.path)).toEqual(["13-engenharia/a.md"]);
    expect(index.nearest(vec(1, 0), 5, { folder: "13-engenharia//" }).map((r) => r.path)).toEqual(["13-engenharia/a.md"]);
  });

  it("nearest devolve no máximo k notas e nunca corta no meio de uma nota", () => {
    const index = new VectorIndex();
    for (let i = 0; i < 4; i++) {
      index.replaceNote(`n/${i}.md`, [
        unit(`n/${i}.md`, { lineStart: 1, vector: vec(1, 0) }),
        unit(`n/${i}.md`, { lineStart: 9, vector: vec(0, 1) })
      ]);
    }
    const result = index.nearest(vec(0, 1), 2);
    expect(result).toHaveLength(2);
    expect(new Set(result.map((r) => r.path)).size).toBe(2);
  });

  it("replaceNote troca todos os vetores da nota", () => {
    const index = new VectorIndex();
    index.replaceNote("a/nota.md", [
      unit("a/nota.md", { lineStart: 1, textHash: "velho-1" }),
      unit("a/nota.md", { lineStart: 9, textHash: "velho-2" })
    ]);
    index.replaceNote("a/nota.md", [unit("a/nota.md", { lineStart: 1, textHash: "novo-1", vector: vec(0, 1) })]);

    expect(index.size()).toBe(1);
    expect(index.hashes("a/nota.md")).toEqual(new Set(["novo-1"]));
    expect(index.nearest(vec(0, 1), 5)[0]).toMatchObject({ path: "a/nota.md", score: 1 });
  });

  it("renameNote move e deleteNote apaga", () => {
    const index = new VectorIndex();
    index.replaceNote("velha/pasta/nota.md", [unit("velha/pasta/nota.md", { vector: vec(1, 0) })]);
    index.replaceNote("outra/lixo.md", [unit("outra/lixo.md", { vector: vec(0, 1) })]);

    index.renameNote("velha/pasta/nota.md", "nova/nota.md");

    expect(index.hashes("velha/pasta/nota.md")).toEqual(new Set());
    expect(index.hashes("nova/nota.md").size).toBe(1);
    expect(index.pathOfStem("nota")).toBe("nova/nota.md");
    expect(index.nearest(vec(1, 0), 1, { folder: "nova" }).map((r) => r.path)).toEqual(["nova/nota.md"]);
    expect(index.nearest(vec(1, 0), 1, { folder: "velha" })).toEqual([]);

    index.deleteNote("outra/lixo.md");

    expect(index.size()).toBe(1);
    expect(index.hashes("outra/lixo.md")).toEqual(new Set());
    expect(index.pathOfStem("lixo")).toBeNull();
  });

  it("pathOfStem acha o caminho pelo nome do arquivo", () => {
    const index = new VectorIndex();
    index.replaceNote("02-marcas/madame/Projetos/reuniao.md", [unit("02-marcas/madame/Projetos/reuniao.md")]);
    index.replaceNote("06-tarefas/Tasks/outra.md", [unit("06-tarefas/Tasks/outra.md")]);

    expect(index.pathOfStem("reuniao")).toBe("02-marcas/madame/Projetos/reuniao.md");
    expect(index.pathOfStem("inexistente")).toBeNull();
  });

  it("pathOfStem escolhe a pasta mais rasa quando dois arquivos têm o mesmo nome", () => {
    const index = new VectorIndex();
    index.replaceNote("fundo/profundo/ideia.md", [unit("fundo/profundo/ideia.md")]);
    index.replaceNote("ideia.md", [unit("ideia.md")]);

    expect(index.pathOfStem("ideia")).toBe("ideia.md");
  });
});

type Row = Record<string, unknown>;

/**
 * Banco falso das duas tabelas de vetores: guarda as linhas, responde aos SQL que
 * a PgVectorStore usa e registra cada comando com os parâmetros enviados.
 */
function fakeVectorDb() {
  const vectors: Row[] = [];
  const states: Row[] = [];
  const sent: Array<{ text: string; values: unknown[] }> = [];

  const run = async (text: string, values: unknown[] = []): Promise<{ rows: Row[]; rowCount: number }> => {
    sent.push({ text, values });
    const sql = text.replace(/\s+/g, " ").trim();
    const sameKey = (row: Row) => row.collection_id === values[0] && row.model === values[1];
    if (/^(BEGIN|COMMIT|ROLLBACK)$/.test(sql)) return { rows: [], rowCount: 0 };
    if (sql.startsWith("INSERT INTO mcp_embedding_vectors")) {
      const columns = /\(([^)]*)\) VALUES/.exec(sql)![1].split(",").map((c) => c.trim());
      for (let i = 0; i < values.length; i += columns.length) {
        const row: Row = {};
        columns.forEach((column, j) => (row[column] = values[i + j]));
        const key = (r: Row) => [r.collection_id, r.model, r.path, r.kind, r.seq].join("|");
        if (vectors.some((v) => key(v) === key(row))) throw new Error("duplicate key value violates unique constraint");
        vectors.push(row);
      }
      return { rows: [], rowCount: values.length / columns.length };
    }
    if (sql.startsWith("DELETE FROM mcp_embedding_vectors")) {
      const before = vectors.length;
      for (let i = vectors.length - 1; i >= 0; i--) {
        if (sameKey(vectors[i]) && vectors[i].path === values[2]) vectors.splice(i, 1);
      }
      return { rows: [], rowCount: before - vectors.length };
    }
    if (sql.startsWith("UPDATE mcp_embedding_vectors")) {
      let moved = 0;
      for (const row of vectors) {
        if (sameKey(row) && row.path === values[2]) {
          row.path = values[3];
          row.folder = values[4];
          moved++;
        }
      }
      return { rows: [], rowCount: moved };
    }
    if (sql.startsWith("SELECT path, kind")) {
      return { rows: vectors.filter(sameKey), rowCount: 0 };
    }
    if (sql.startsWith("SELECT cursor FROM mcp_embedding_state")) {
      const row = states.find(sameKey);
      return { rows: row ? [{ cursor: row.cursor }] : [], rowCount: row ? 1 : 0 };
    }
    if (sql.startsWith("INSERT INTO mcp_embedding_state")) {
      const row = states.find(sameKey);
      if (row) row.cursor = values[2];
      else states.push({ collection_id: values[0], model: values[1], cursor: values[2] });
      return { rows: [], rowCount: 1 };
    }
    throw new Error(`SQL não previsto no banco falso: ${sql}`);
  };

  return {
    vectors,
    sent,
    query: run,
    async connect() {
      return { query: run, release() {} };
    }
  };
}

describe("PgVectorStore", () => {
  it("grava e relê os vetores sem texto", async () => {
    const db = fakeVectorDb();
    const store = new PgVectorStore(db as never, COLLECTION, MODEL);
    const path = "13-engenharia/01-planejamento/plano.md";

    await store.saveNote(path, [
      unit(path, {
        kind: "chunk",
        lineStart: 3,
        lineEnd: 40,
        textHash: "h-chunk",
        revision: "rev-9",
        types: ["task", "ops"],
        vector: vec(0.6, 0.8)
      }),
      unit(path, {
        kind: "card",
        lineStart: 0,
        lineEnd: 0,
        textHash: "h-card",
        revision: "rev-9",
        types: ["task", "ops"],
        vector: vec(1, 0)
      })
    ]);
    await store.setCursor("cursor-42");

    const reloaded = await new PgVectorStore(db as never, COLLECTION, MODEL).load();

    expect(reloaded.size()).toBe(2);
    expect(reloaded.hashes(path)).toEqual(new Set(["h-chunk", "h-card"]));
    const best = reloaded.nearest(vec(0.6, 0.8), 1, { folder: "13-engenharia/01-planejamento" });
    expect(best).toHaveLength(1);
    expect(best[0]).toMatchObject({ path, lineStart: 3, lineEnd: 40, kind: "chunk" });
    expect(best[0].score).toBeCloseTo(1, 5);
    expect(await new PgVectorStore(db as never, COLLECTION, MODEL).cursor()).toBe("cursor-42");
    expect(await new PgVectorStore(db as never, COLLECTION, "outro-modelo").cursor()).toBeNull();
  });

  it("load marca o índice como pronto só quando há cursor", async () => {
    const db = fakeVectorDb();
    const store = new PgVectorStore(db as never, COLLECTION, MODEL);
    await store.saveNote("a/nota.md", [unit("a/nota.md", { textHash: "h-1" })]);

    expect((await store.load()).ready).toBe(false);

    await store.setCursor("c-1");

    expect((await store.load()).ready).toBe(true);
  });

  it("nenhuma coluna recebe texto: só o hash, o vetor e os metadados", async () => {
    const db = fakeVectorDb();
    const store = new PgVectorStore(db as never, COLLECTION, MODEL);

    await store.saveNote("a/nota.md", [unit("a/nota.md", { textHash: "h-1" })]);

    const insert = db.sent.find((s) => s.text.includes("INSERT INTO mcp_embedding_vectors"));
    expect(insert).toBeDefined();
    const columns = /\(([^)]*)\)\s+VALUES/.exec(insert!.text)![1].split(",").map((c) => c.trim());
    expect(columns).toEqual([
      "collection_id", "model", "path", "kind", "seq", "line_start", "line_end",
      "text_hash", "revision", "types", "folder", "vector"
    ]);
    expect(columns).not.toContain("text");
    expect(columns).not.toContain("heading_hash");
    expect(Buffer.isBuffer(db.vectors[0].vector)).toBe(true);
    expect((db.vectors[0].vector as Buffer).byteLength).toBe(8);
  });

  it("unidades de uma linha longa, com o mesmo lineStart, gravam com seq diferente", async () => {
    const db = fakeVectorDb();
    const store = new PgVectorStore(db as never, COLLECTION, MODEL);

    await store.saveNote("a/nota.md", [
      unit("a/nota.md", { seq: 1, lineStart: 3, lineEnd: 3, textHash: "h-1" }),
      unit("a/nota.md", { seq: 2, lineStart: 3, lineEnd: 3, textHash: "h-2" })
    ]);

    expect(db.vectors.map((r) => r.seq)).toEqual([1, 2]);
    expect((await new PgVectorStore(db as never, COLLECTION, MODEL).load()).size()).toBe(2);
  });

  it("renameNote e deleteNote mudam e apagam as linhas do banco", async () => {
    const db = fakeVectorDb();
    const store = new PgVectorStore(db as never, COLLECTION, MODEL);
    await store.saveNote("velha/nota.md", [unit("velha/nota.md", { textHash: "h-1" })]);

    await store.renameNote("velha/nota.md", "nova/pasta/nota.md");

    expect(db.vectors.map((r) => [r.path, r.folder])).toEqual([["nova/pasta/nota.md", "nova/pasta"]]);

    await store.deleteNote("nova/pasta/nota.md");

    expect(db.vectors).toEqual([]);
  });
});

describe("migrate das tabelas de vetores", () => {
  it("cria as duas tabelas sem coluna de texto nem heading_hash", async () => {
    const db = await createDatabase("memory");

    const { rows } = await db.query<{ column_name: string }>(
      "SELECT column_name FROM information_schema.columns WHERE table_name = 'mcp_embedding_vectors'"
    );
    const names = rows.map((r) => r.column_name);

    expect(names).toContain("vector");
    expect(names).toContain("folder");
    expect(names).toContain("seq");
    expect(names).not.toContain("text");
    expect(names).not.toContain("heading_hash");
    await expect(db.query("SELECT cursor FROM mcp_embedding_state")).resolves.toBeDefined();
  });
});
