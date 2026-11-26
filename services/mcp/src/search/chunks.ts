import { createHash } from "node:crypto";

/** A piece of text sent to the embedding model: the note card or one body chunk. */
export interface EmbedUnit {
  kind: "card" | "chunk";
  /** Order of the unit in the note: 0 for the card, 1.. for the chunks. Unique within a note. */
  seq: number;
  text: string;
  /** 1-based body line numbers, as read_record counts them; 0 for the card. */
  lineStart: number;
  lineEnd: number;
  /** Heading trail ("A > B") or null when the text is above the first heading. */
  heading: string | null;
  /** sha256 hex of `text`. */
  hash: string;
}

export const CHUNK_MIN_CHARS = 1024;
export const CHUNK_MAX_CHARS = 2048;

const HEADING = /^(#{1,6})\s+(\S.*?)\s*$/;
const FENCE_OPEN = /^\s{0,3}(`{3,}|~{3,})/;
const FENCE_CLOSE = /^\s{0,3}(`{3,}|~{3,})\s*$/;
const WIKILINK = /\[\[([^\]|]+)(?:\|[^\]]*)?\]\]/g;

/** A contiguous run of body lines, 0-based start and exclusive end. */
interface Section {
  start: number;
  end: number;
  trail: string | null;
}

/** A piece of body text with its 1-based inclusive line range. */
interface Piece {
  text: string;
  start: number;
  end: number;
}

function sha256(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

function unit(kind: EmbedUnit["kind"], seq: number, text: string, lineStart: number, lineEnd: number, heading: string | null): EmbedUnit {
  return { kind, seq, text, lineStart, lineEnd, heading, hash: sha256(text) };
}

/** Collapses a scalar to one clean line, turning [[x]] and [[x|alias]] into x. */
function clean(value: string): string {
  return value.replace(WIKILINK, "$1").replace(/\s+/g, " ").trim();
}

/** Flattens a frontmatter value into clean, non-empty strings. */
function values(value: unknown): string[] {
  if (value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap(values);
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    const text = clean(String(value));
    return text ? [text] : [];
  }
  return [];
}

/** Title from frontmatter, or the file name without `.md`. */
function titleOf(path: string, frontmatter: Record<string, unknown>): string {
  const [title] = values(frontmatter.title);
  if (title) return title;
  const name = path.split("/").filter(Boolean).pop() ?? path;
  return name.replace(/\.md$/, "");
}

/** Area: the first two folders of the path, joined by "/". */
function areaOf(path: string): string {
  return path.split("/").filter(Boolean).slice(0, -1).slice(0, 2).join("/");
}

export function noteCard(path: string, frontmatter: Record<string, unknown>): EmbedUnit {
  const type = values(frontmatter.type);
  const fields: Array<[string, string[]]> = [
    ["Tipo", type.length > 0 ? type : values(frontmatter.types)],
    ["Marca", values(frontmatter.brand)],
    ["Área", [areaOf(path)].filter(Boolean)],
    ["Título", [titleOf(path, frontmatter)]],
    ["Descrição", values(frontmatter.description)],
    ["Tags", values(frontmatter.tags)],
    ["Projetos", values(frontmatter.projects)],
    ["Status", values(frontmatter.status)],
  ];
  const lines = fields
    .filter(([, items]) => items.length > 0)
    .map(([label, items]) => `${label}: ${items.join(", ")}`);
  return unit("card", 0, lines.join("\n"), 0, 0, null);
}

/** Splits the body into sections at Markdown headings, ignoring headings inside code fences. */
function splitSections(lines: string[]): Section[] {
  const boundaries: Array<{ index: number; trail: string | null }> = [{ index: 0, trail: null }];
  const stack: Array<{ level: number; title: string }> = [];
  let fence: string | null = null;

  lines.forEach((line, index) => {
    if (fence !== null) {
      const close = FENCE_CLOSE.exec(line);
      if (close && close[1][0] === fence[0] && close[1].length >= fence.length) fence = null;
      return;
    }
    const open = FENCE_OPEN.exec(line);
    if (open) {
      fence = open[1];
      return;
    }
    const heading = HEADING.exec(line);
    if (!heading) return;
    const level = heading[1].length;
    while (stack.length > 0 && stack[stack.length - 1].level >= level) stack.pop();
    stack.push({ level, title: heading[2].replace(/\s+#+$/, "") });
    const trail = stack.map((s) => s.title).join(" > ");
    const last = boundaries[boundaries.length - 1];
    if (last.index === index) boundaries[boundaries.length - 1] = { index, trail };
    else boundaries.push({ index, trail });
  });

  const sections: Section[] = [];
  boundaries.forEach((boundary, k) => {
    let start = boundary.index;
    let end = k + 1 < boundaries.length ? boundaries[k + 1].index : lines.length;
    while (start < end && lines[start].trim() === "") start++;
    while (end > start && lines[end - 1].trim() === "") end--;
    if (start < end) sections.push({ start, end, trail: boundary.trail });
  });
  return sections;
}

function textOf(lines: string[], start: number, end: number): string {
  return lines.slice(start, end).join("\n");
}

/** Joins a short section with the following ones while it stays under the minimum and the merge fits the maximum. */
function mergeShort(lines: string[], sections: Section[]): Section[] {
  const merged: Section[] = [];
  for (let i = 0; i < sections.length; i++) {
    const current = { ...sections[i] };
    while (
      textOf(lines, current.start, current.end).length < CHUNK_MIN_CHARS &&
      i + 1 < sections.length &&
      textOf(lines, current.start, sections[i + 1].end).length <= CHUNK_MAX_CHARS
    ) {
      i++;
      current.end = sections[i].end;
    }
    merged.push(current);
  }
  return merged;
}

/** Greedily packs items into pieces of at most CHUNK_MAX_CHARS, joined by `separator`. */
function pack(items: Piece[], separator: string): Piece[] {
  const out: Piece[] = [];
  let current: Piece | null = null;
  for (const item of items) {
    if (current && current.text.length + separator.length + item.text.length <= CHUNK_MAX_CHARS) {
      current = { text: current.text + separator + item.text, start: current.start, end: item.end };
    } else {
      if (current) out.push(current);
      current = { ...item };
    }
  }
  if (current) out.push(current);
  return out;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

/** Cuts a single line into pieces of at most CHUNK_MAX_CHARS characters. */
function cutLine(line: string, lineNumber: number): Piece[] {
  const pieces: Piece[] = [];
  for (let offset = 0; offset < line.length; ) {
    let end = Math.min(offset + CHUNK_MAX_CHARS, line.length);
    // Never split a surrogate pair (an emoji) between two pieces.
    if (end < line.length && isHighSurrogate(line.charCodeAt(end - 1))) end -= 1;
    pieces.push({ text: line.slice(offset, end), start: lineNumber, end: lineNumber });
    offset = end;
  }
  return pieces;
}

/** Splits an oversized section by paragraph; a paragraph still too big is split by line, and a line by characters. */
function splitByParagraph(lines: string[], start: number, end: number): Piece[] {
  const groups: Array<{ start: number; end: number }> = [];
  let i = start;
  while (i < end) {
    if (lines[i].trim() === "") {
      i++;
      continue;
    }
    let j = i;
    while (j < end && lines[j].trim() !== "") j++;
    groups.push({ start: i, end: j });
    i = j;
  }

  const pieces: Piece[] = [];
  let small: Piece[] = [];
  const flushSmall = () => {
    pieces.push(...pack(small, "\n\n"));
    small = [];
  };
  for (const group of groups) {
    const text = textOf(lines, group.start, group.end);
    const item: Piece = { text, start: group.start + 1, end: group.end };
    if (text.length <= CHUNK_MAX_CHARS) {
      small.push(item);
      continue;
    }
    flushSmall();
    const lineItems: Piece[] = [];
    for (let k = group.start; k < group.end; k++) {
      const line = lines[k];
      if (line.length <= CHUNK_MAX_CHARS) lineItems.push({ text: line, start: k + 1, end: k + 1 });
      else lineItems.push(...cutLine(line, k + 1));
    }
    pieces.push(...pack(lineItems, "\n"));
  }
  flushSmall();
  return pieces;
}

export function bodyChunks(path: string, title: string, body: string): EmbedUnit[] {
  const lines = body.split(/\r?\n/);
  const sections = mergeShort(lines, splitSections(lines));
  const prefixTitle = clean(title);
  const area = areaOf(path);
  const units: EmbedUnit[] = [];

  for (const section of sections) {
    const text = textOf(lines, section.start, section.end);
    const pieces: Piece[] =
      text.length <= CHUNK_MAX_CHARS
        ? [{ text, start: section.start + 1, end: section.end }]
        : splitByParagraph(lines, section.start, section.end);
    for (const piece of pieces) {
      const label = [prefixTitle, area, section.trail].filter(Boolean).join(" — ");
      units.push(unit("chunk", units.length + 1, `${label}\n\n${piece.text}`, piece.start, piece.end, section.trail));
    }
  }
  return units;
}

export function embedUnits(path: string, frontmatter: Record<string, unknown>, body: string): EmbedUnit[] {
  return [noteCard(path, frontmatter), ...bodyChunks(path, titleOf(path, frontmatter), body)];
}
