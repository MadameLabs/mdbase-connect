import type { ConnectGateway } from "../connect.js";
import { celString } from "./candidates.js";

export const EXCERPT_MAX_CHARS = 1_200;

export interface Excerpt {
  text: string;
  lineStart: number;
  lineEnd: number;
  heading: string | null;
}

interface Section {
  heading: string | null;
  /** 0-based, inclusive start and exclusive end, in body lines */
  start: number;
  end: number;
}

const HEADING = /^#{1,6} +(.*?)(?:\s+#+)?\s*$/;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;

function splitSections(lines: string[]): Section[] {
  const sections: Section[] = [];
  let current: Section = { heading: null, start: 0, end: 0 };
  let fence: string | null = null;
  lines.forEach((line, i) => {
    const f = FENCE.exec(line);
    if (f) {
      if (fence === null) fence = f[1][0];
      else if (f[1][0] === fence) fence = null;
    } else if (fence === null) {
      const h = HEADING.exec(line);
      if (h) {
        current.end = i;
        sections.push(current);
        current = { heading: h[1], start: i, end: i };
      }
    }
  });
  current.end = lines.length;
  sections.push(current);
  return sections;
}

function countHits(text: string, terms: readonly string[]): number {
  const lower = text.toLowerCase();
  let total = 0;
  for (const term of terms) {
    if (!term) continue;
    for (let at = lower.indexOf(term); at !== -1; at = lower.indexOf(term, at + term.length)) total += 1;
  }
  return total;
}

function cut(lines: string[], from: number, to: number, terms: readonly string[]): { text: string; start: number; end: number } {
  const whole = lines.slice(from, to).join("\n");
  if (whole.length <= EXCERPT_MAX_CHARS) return { text: whole, start: from, end: to };
  let center = from;
  for (let i = from; i < to; i++) {
    if (countHits(lines[i], terms) > 0) {
      center = i;
      break;
    }
  }
  if (lines[center].length > EXCERPT_MAX_CHARS) {
    const lower = lines[center].toLowerCase();
    const first = terms.filter(Boolean).map((t) => lower.indexOf(t)).filter((n) => n >= 0).sort((a, b) => a - b)[0] ?? 0;
    const begin = Math.max(0, Math.min(first - Math.floor(EXCERPT_MAX_CHARS / 2), lines[center].length - EXCERPT_MAX_CHARS));
    return { text: lines[center].slice(begin, begin + EXCERPT_MAX_CHARS), start: center, end: center + 1 };
  }
  let a = center;
  let b = center + 1;
  let size = lines[center].length;
  for (;;) {
    let grew = false;
    if (a > from && size + 1 + lines[a - 1].length <= EXCERPT_MAX_CHARS) {
      a -= 1;
      size += 1 + lines[a].length;
      grew = true;
    }
    if (b < to && size + 1 + lines[b].length <= EXCERPT_MAX_CHARS) {
      size += 1 + lines[b].length;
      b += 1;
      grew = true;
    }
    if (!grew) break;
  }
  return { text: lines.slice(a, b).join("\n"), start: a, end: b };
}

/** The section with most term hits (ties: the first; no hit: the first non-empty one), cut to EXCERPT_MAX_CHARS. */
export function bestExcerpt(body: string, rawTerms: readonly string[]): Excerpt | null {
  const terms = rawTerms.map((t) => t.toLowerCase());
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  let best: { section: Section; start: number; end: number; hits: number } | null = null;
  for (const section of splitSections(lines)) {
    let start = section.start;
    let end = section.end;
    while (start < end && lines[start].trim() === "") start += 1;
    while (end > start && lines[end - 1].trim() === "") end -= 1;
    if (start >= end) continue;
    const hits = countHits(lines.slice(start, end).join("\n"), terms);
    if (!best || hits > best.hits) best = { section, start, end, hits };
  }
  if (!best) return null;
  const { text, start, end } = cut(lines, best.start, best.end, terms);
  return { text, lineStart: start + 1, lineEnd: end, heading: best.section.heading };
}

/** The body lines `lineStart..lineEnd` (1-based, inclusive), counted like the vector chunks are: split on `\r?\n`. */
export function excerptAt(body: string, lineStart: number, lineEnd: number): Excerpt {
  const lines = body.split(/\r?\n/);
  const start = Math.min(Math.max(1, lineStart), lines.length);
  const end = Math.min(Math.max(start, lineEnd), lines.length);
  const section = splitSections(lines).find((s) => start - 1 >= s.start && start - 1 < s.end);
  return { text: lines.slice(start - 1, end).join("\n"), lineStart: start, lineEnd: end, heading: section?.heading ?? null };
}

export interface NoteRecord {
  body: string;
  frontmatter: Record<string, unknown>;
}

/** One `query` for all paths, with bodies and frontmatter. Missing or non-string bodies are skipped. */
export async function readRecords(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  paths: readonly string[],
  signal?: AbortSignal
): Promise<Map<string, NoteRecord>> {
  const records = new Map<string, NoteRecord>();
  if (paths.length === 0) return records;
  const where = `file.path in [${paths.map(celString).join(", ")}]`;
  const page = await gateway.operation(
    connectionSetId,
    connectionId,
    "query",
    { where, include_body: true, limit: paths.length },
    { signal }
  );
  const results = (page as { results?: unknown } | null)?.results;
  if (!Array.isArray(results)) return records;
  for (const item of results) {
    const r = item as { path?: unknown; body?: unknown; frontmatter?: unknown };
    if (typeof r?.path !== "string" || typeof r.body !== "string") continue;
    const fm = r.frontmatter;
    records.set(r.path, { body: r.body, frontmatter: fm !== null && typeof fm === "object" && !Array.isArray(fm) ? (fm as Record<string, unknown>) : {} });
  }
  return records;
}

/** Bodies only: a thin view over readRecords. */
export async function readBodies(
  gateway: ConnectGateway,
  connectionSetId: string,
  connectionId: string,
  paths: readonly string[],
  signal?: AbortSignal
): Promise<Map<string, string>> {
  const records = await readRecords(gateway, connectionSetId, connectionId, paths, signal);
  return new Map([...records].map(([path, record]) => [path, record.body]));
}
