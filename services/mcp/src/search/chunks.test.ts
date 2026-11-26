import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { CHUNK_MAX_CHARS, CHUNK_MIN_CHARS, bodyChunks, embedUnits, noteCard } from "./chunks.js";

/** Texto do trecho sem o prefixo "<título> — <pasta> — <trilha>\n\n". */
function semPrefixo(text: string): string {
  return text.slice(text.indexOf("\n\n") + 2);
}

describe("noteCard", () => {
  it("cartão segue a ordem e omite o que falta", () => {
    const u = noteCard("13-engenharia/01-planejamento/x.md", { type: "design", brand: "[[madame-labs]]", description: "D", tags: ["a", "b"] });
    expect(u.text).toBe("Tipo: design\nMarca: madame-labs\nÁrea: 13-engenharia/01-planejamento\nTítulo: x\nDescrição: D\nTags: a, b");
    expect(u).toMatchObject({ kind: "card", lineStart: 0, lineEnd: 0, heading: null });
  });

  it("usa title do frontmatter, types como tipo, projects e status, e normaliza [[x|apelido]]", () => {
    const u = noteCard("02-marcas/ml/Projetos/p.md", {
      title: "Projeto P",
      types: ["projeto", "[[marca]]"],
      projects: ["[[alfa|Alfa]]", "beta"],
      status: "ativo",
    });
    expect(u.text).toBe(
      "Tipo: projeto, marca\nÁrea: 02-marcas/ml\nTítulo: Projeto P\nProjetos: alfa, beta\nStatus: ativo"
    );
  });

  it("sem pasta no caminho nem campos, sobra só o título do nome do arquivo", () => {
    expect(noteCard("solta.md", {}).text).toBe("Título: solta");
  });

  it("valores vazios não aparecem na linha", () => {
    expect(noteCard("a/b/c.md", { title: "T", tags: [], description: "  ", status: null }).text).toBe(
      "Área: a/b\nTítulo: T"
    );
  });
});

describe("bodyChunks", () => {
  it("trechos cortam por seção, com prefixo e linhas do corpo", () => {
    const body = ["# A", "a".repeat(1100), "", "## B", "b".repeat(1100), ""].join("\n");
    const units = bodyChunks("13-engenharia/x.md", "t", body);
    expect(units).toHaveLength(2);
    expect(units[0]).toMatchObject({ kind: "chunk", lineStart: 1, lineEnd: 2, heading: "A" });
    expect(units[0].text).toBe(`t — 13-engenharia — A\n\n# A\n${"a".repeat(1100)}`);
    expect(units[1]).toMatchObject({ kind: "chunk", lineStart: 4, lineEnd: 5, heading: "A > B" });
    expect(units[1].text).toBe(`t — 13-engenharia — A > B\n\n## B\n${"b".repeat(1100)}`);
  });

  it("seção curta junta à seguinte; seção longa divide por parágrafo", () => {
    const p = (c: string) => c.repeat(1000);
    const body = [
      "# A",
      "a".repeat(200),
      "",
      "## B",
      "b".repeat(1500),
      "",
      "## C",
      p("c"),
      "",
      p("d"),
      "",
      p("e"),
    ].join("\n");
    const units = bodyChunks("13-engenharia/x.md", "t", body);
    expect(units).toHaveLength(3);
    for (const u of units) {
      expect(semPrefixo(u.text).length).toBeLessThanOrEqual(CHUNK_MAX_CHARS);
    }
    // A (curta) juntou-se a B e passou do mínimo; C não coube no mesmo trecho.
    expect(units[0].heading).toBe("A");
    expect(units[0].text).toContain("## B");
    expect(units[0].text).not.toContain("## C");
    expect(units[1].heading).toBe("A > C");
    expect(units[1].lineStart).toBe(7);
    // O trecho de C dividido por parágrafo: o terceiro parágrafo fica sozinho.
    expect(units[2].heading).toBe("A > C");
    expect(semPrefixo(units[2].text)).toBe(p("e"));
    expect(semPrefixo(units[0].text).length).toBeGreaterThanOrEqual(CHUNK_MIN_CHARS);
  });

  it("título dentro de bloco de código não corta", () => {
    const body = ["# Real", "r".repeat(1100), "```", "# não é título", "```", ""].join("\n");
    const units = bodyChunks("13-engenharia/x.md", "t", body);
    expect(units).toHaveLength(1);
    expect(units[0].heading).toBe("Real");
    expect(units[0].text).toContain("```\n# não é título\n```");
  });

  it("trilha desce e volta: título de nível igual ou acima fecha o ramo anterior", () => {
    const body = ["# A", "a".repeat(1100), "## B", "b".repeat(1100), "# C", "c".repeat(1100)].join("\n");
    const units = bodyChunks("13-engenharia/x.md", "t", body);
    expect(units.map((u) => u.heading)).toEqual(["A", "A > B", "C"]);
  });

  it("texto antes do primeiro título tem heading null e prefixo sem trilha", () => {
    const body = ["intro " + "i".repeat(1100), "# A", "x"].join("\n");
    const units = bodyChunks("solta.md", "t", body);
    expect(units[0]).toMatchObject({ heading: null, lineStart: 1, lineEnd: 1 });
    expect(units[0].text.startsWith("t\n\n")).toBe(true);
  });
});

describe("embedUnits", () => {
  it("nota sem corpo gera só o cartão", () => {
    expect(embedUnits("a.md", { title: "A" }, "")).toHaveLength(1);
    expect(embedUnits("a.md", { title: "A" }, "\n  \n")).toHaveLength(1);
  });

  it("cartão vem primeiro e os trechos usam o título do frontmatter", () => {
    const units = embedUnits("13-engenharia/x.md", { title: "Título X" }, "# A\nvalor curto");
    expect(units.map((u) => u.kind)).toEqual(["card", "chunk"]);
    expect(units[1].text).toBe("Título X — 13-engenharia — A\n\n# A\nvalor curto");
    expect(units[1]).toMatchObject({ lineStart: 1, lineEnd: 2 });
  });
});

describe("hash", () => {
  it("hash é sha256 hex do texto, estável e muda com o texto", () => {
    const a = noteCard("x.md", { description: "D" });
    const b = noteCard("x.md", { description: "D" });
    const c = noteCard("x.md", { description: "D!" });
    expect(a.hash).toBe(b.hash);
    expect(a.hash).not.toBe(c.hash);
    expect(a.hash).toBe(createHash("sha256").update(a.text, "utf8").digest("hex"));
    expect(a.hash).toMatch(/^[0-9a-f]{64}$/);
  });
});
