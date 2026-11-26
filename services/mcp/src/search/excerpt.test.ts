import { describe, expect, it, vi } from "vitest";
import type { ConnectGateway } from "../connect.js";
import { EXCERPT_MAX_CHARS, bestExcerpt, excerptAt, readBodies, readRecords } from "./excerpt.js";

describe("bestExcerpt", () => {
  it("escolhe a seção com o termo, com título e linhas corretas", () => {
    const body = ["intro", "", "# Um", "nada aqui", "# Dois", "fala de cartão", "e mais cartão", "# Três", "fim"].join("\n");
    const e = bestExcerpt(body, ["cartão"]);
    expect(e).toEqual({ text: "# Dois\nfala de cartão\ne mais cartão", heading: "Dois", lineStart: 5, lineEnd: 7 });
  });
  it("ignora maiúsculas, desempata pela primeira e usa a primeira não vazia sem acerto", () => {
    expect(bestExcerpt("# A\nCartão\n# B\ncartão", ["cartão"])?.heading).toBe("A");
    const e = bestExcerpt("\n\n# A\nxis\n# B\nyyy", ["zzz"]);
    expect(e?.heading).toBe("A");
    expect(e?.lineStart).toBe(3);
  });
  it("texto antes do primeiro título tem heading null", () => {
    const e = bestExcerpt("solto cartão\n# A\nx", ["cartão"]);
    expect(e).toEqual({ text: "solto cartão", heading: null, lineStart: 1, lineEnd: 1 });
  });
  it("ignora títulos dentro de bloco de código", () => {
    const body = "# A\n```\n# não é título\n```\ncartão\n~~~\n## nem este\n~~~";
    const e = bestExcerpt(body, ["cartão"]);
    expect(e?.heading).toBe("A");
    expect(e?.lineEnd).toBe(8);
  });
  it("corta seção grande em 1.200, centrado no primeiro acerto, com linhas exatas", () => {
    const lines = Array.from({ length: 100 }, (_, i) => `linha ${i + 1} ${"x".repeat(40)}`);
    lines[59] = "linha 60 ALVO";
    const body = ["# Grande", ...lines].join("\n");
    const e = bestExcerpt(body, ["alvo"]);
    expect(e).not.toBeNull();
    expect(e!.text.length).toBeLessThanOrEqual(EXCERPT_MAX_CHARS);
    expect(e!.text).toContain("ALVO");
    expect(e!.text.split("\n")).toHaveLength(e!.lineEnd - e!.lineStart + 1);
    expect(body.split("\n").slice(e!.lineStart - 1, e!.lineEnd).join("\n")).toBe(e!.text);
    const target = 61;
    expect(Math.abs((e!.lineStart + e!.lineEnd) / 2 - target)).toBeLessThan(5);
  });
  it("uma linha única maior que o teto é cortada em caracteres", () => {
    const e = bestExcerpt("a".repeat(5000), ["a"]);
    expect(e!.text.length).toBe(EXCERPT_MAX_CHARS);
    expect([e!.lineStart, e!.lineEnd]).toEqual([1, 1]);
  });
  it("corpo vazio ou só espaços devolve null", () => {
    expect(bestExcerpt("", ["a"])).toBeNull();
    expect(bestExcerpt("  \n\n", ["a"])).toBeNull();
  });
  it("corpo de 300 KB devolve trecho dentro do teto", () => {
    const body = "# T\n" + "palavra qualquer de enchimento\n".repeat(10_000) + "cartão\n";
    const e = bestExcerpt(body, ["cartão"]);
    expect(e!.text.length).toBeLessThanOrEqual(EXCERPT_MAX_CHARS);
    expect(e!.text).toContain("cartão");
  });
});

describe("bestExcerpt, correções", () => {
  it("termo com maiúscula acha a seção", () => {
    expect(bestExcerpt("# A\nnada\n# B\ncartão", ["Cartão"])?.heading).toBe("B");
  });
  it("CRLF e CR solto viram \\n", () => {
    const e = bestExcerpt("# A\r\nlinha cartão\r\nmais\rfim", ["cartão"]);
    expect(e!.text).toBe("# A\nlinha cartão\nmais\nfim");
    expect(e!.lineEnd).toBe(4);
  });
  it("# final só sai se houver espaço antes", () => {
    expect(bestExcerpt("# C#\nx", ["x"])?.heading).toBe("C#");
    expect(bestExcerpt("## Título ##\nx", ["x"])?.heading).toBe("Título");
  });
});

describe("readBodies", () => {
  const make = (results: unknown) => {
    const operation = vi.fn(async () => ({ results }));
    return { gateway: { operation } as unknown as ConnectGateway, operation };
  };
  it("faz uma query só para 30 caminhos", async () => {
    const paths = Array.from({ length: 30 }, (_, i) => `n${i}.md`);
    const { gateway, operation } = make(paths.map((path) => ({ path, body: `corpo ${path}` })));
    const map = await readBodies(gateway, "s", "c", paths);
    expect(operation).toHaveBeenCalledTimes(1);
    const call = operation.mock.calls[0] as unknown as [string, string, string, Record<string, unknown>];
    expect(call[2]).toBe("query");
    expect(call[3]).toMatchObject({ include_body: true, limit: 30 });
    expect(call[3].where).toContain('file.path in ["n0.md", "n1.md"');
    expect(map.size).toBe(30);
    expect(map.get("n3.md")).toBe("corpo n3.md");
  });
  it("escapa aspas e pula corpo ausente", async () => {
    const { gateway, operation } = make([{ path: 'a"b.md', body: "x" }, { path: "c.md" }, { path: "d.md", body: 5 }]);
    const map = await readBodies(gateway, "s", "c", ['a"b.md', "c.md", "d.md"]);
    expect((operation.mock.calls[0] as unknown as unknown[])[3]).toMatchObject({ where: 'file.path in ["a\\"b.md", "c.md", "d.md"]' });
    expect([...map.keys()]).toEqual(['a"b.md']);
  });
  it("sem caminhos não chama o gateway", async () => {
    const { gateway, operation } = make([]);
    expect((await readBodies(gateway, "s", "c", [])).size).toBe(0);
    expect(operation).not.toHaveBeenCalled();
  });
  it("propaga erro do gateway", async () => {
    const gateway = { operation: vi.fn(async () => { throw new Error("falhou"); }) } as unknown as ConnectGateway;
    await expect(readBodies(gateway, "s", "c", ["a.md"])).rejects.toThrow("falhou");
  });
});

describe("excerptAt", () => {
  it("excerptAt devolve as linhas pedidas", () => {
    const e = excerptAt("a\nb\nc\nd", 2, 3);
    expect(e).toMatchObject({ text: "b\nc", lineStart: 2, lineEnd: 3 });
  });
  it("conta linhas como os vetores, com CRLF, e acha o cabeçalho da seção", () => {
    const e = excerptAt("# Um\r\nx\r\n## Dois\r\ny\r\nz", 4, 5);
    expect(e).toMatchObject({ text: "y\nz", lineStart: 4, lineEnd: 5, heading: "Dois" });
  });
  it("corta em EXCERPT_MAX_CHARS, do começo quando não há termos, e ajusta as linhas", () => {
    const body = Array.from({ length: 10 }, (_, i) => `l${i} ${"x".repeat(396)}`).join("\n");
    const e = excerptAt(body, 1, 10);
    expect(e.text.length).toBeLessThanOrEqual(EXCERPT_MAX_CHARS);
    expect(e.lineStart).toBe(1);
    expect(e.lineEnd).toBeLessThan(10);
    expect(e.text.startsWith("l0 ")).toBe(true);
    expect(e.text.split("\n")).toHaveLength(e.lineEnd - e.lineStart + 1);
  });
  it("com termos, centra o recorte onde eles aparecem", () => {
    const lines = Array.from({ length: 10 }, (_, i) => `l${i} ${"x".repeat(396)}`);
    lines[8] = `l8 AGULHA ${"x".repeat(390)}`;
    const e = excerptAt(lines.join("\n"), 1, 10, ["agulha"]);
    expect(e.text.length).toBeLessThanOrEqual(EXCERPT_MAX_CHARS);
    expect(e.text).toContain("AGULHA");
    expect(e.lineStart).toBeGreaterThan(1);
    expect(e.lineStart).toBeLessThanOrEqual(9);
    expect(e.lineEnd).toBeGreaterThanOrEqual(9);
  });
  it("uma linha só maior que o teto também é cortada", () => {
    const e = excerptAt(`a\n${"z".repeat(5_000)}\nb`, 2, 2);
    expect(e.text.length).toBe(EXCERPT_MAX_CHARS);
    expect(e).toMatchObject({ lineStart: 2, lineEnd: 2 });
  });
  it("limita a faixa ao tamanho do corpo", () => {
    const e = excerptAt("a\nb", 2, 9);
    expect(e).toMatchObject({ text: "b", lineStart: 2, lineEnd: 2 });
  });
});

describe("readRecords", () => {
  it("devolve corpo e frontmatter da mesma consulta", async () => {
    const operation = vi.fn(async () => ({ results: [{ path: "a.md", body: "x", frontmatter: { title: "A" } }, { path: "b.md", body: "y" }, { path: "c.md" }] }));
    const gateway = { operation } as unknown as ConnectGateway;
    const map = await readRecords(gateway, "s", "c", ["a.md", "b.md", "c.md"]);
    expect(operation).toHaveBeenCalledTimes(1);
    expect(map.get("a.md")).toEqual({ body: "x", frontmatter: { title: "A" } });
    expect(map.get("b.md")).toEqual({ body: "y", frontmatter: {} });
    expect(map.has("c.md")).toBe(false);
  });
});
