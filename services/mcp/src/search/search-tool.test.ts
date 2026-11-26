import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { describe, expect, it, vi } from "vitest";
import type { CallLog, CallLogEntry } from "../call-log.js";
import type { ConnectGateway } from "../connect.js";
import type { FolderRulesCache } from "../folder-rules.js";
import { createMcpServer, SERVER_INSTRUCTIONS } from "../mcp.js";
import type { OAuthService } from "../oauth.js";
import type { IndexListCache, IndexOption } from "./indexes.js";
import { JevError, type JevClient, type JevRankInput } from "./jev.js";
import { INDEX_MIN_PROBABILITY, JEV_INDEX_BONUS, JEV_MIN_RELEVANCE, SEARCH_RESULT_MAX_CHARS } from "./search-tool.js";

const CONN = "01911111-1111-7111-8111-111111111111";
const COLLECTION = "col-1";

interface Row { path: string; title?: string; description?: string; body?: string }

function fakeGateway(rows: Row[], operations: string[] = ["query", "read"], onOp?: (op: string, input: Record<string, unknown>) => void) {
  const operation = vi.fn(async (_s: string, _c: string, op: string, input: Record<string, unknown>) => {
    onOp?.(op, input);
    if (op !== "query") throw new Error(`unexpected ${op}`);
    const where = String(input.where);
    if (where.startsWith("file.path in [")) {
      return { results: rows.filter((r) => where.includes(JSON.stringify(r.path))).map((r) => ({ path: r.path, body: r.body ?? "" })) };
    }
    return {
      results: rows.map((r) => ({
        path: r.path, types: ["note"], revision: "r1",
        frontmatter: { ...(r.title ? { title: r.title } : {}), ...(r.description ? { description: r.description } : {}) }
      }))
    };
  });
  const listConnections = vi.fn(async () => [
    { id: CONN, collection_id: COLLECTION, display_name: "x", operations, scope: {}, authority: "local" }
  ]);
  return { operation, listConnections } as unknown as ConnectGateway & { operation: typeof operation };
}

function fakeLog() {
  const entries: CallLogEntry[] = [];
  const log: CallLog = { async record(e) { entries.push(e); }, async purgeOlderThan() { return 0; } };
  return { entries, log };
}

async function connect(gateway: ConnectGateway, options: { jev?: JevClient | null; indexes?: IndexOption[] | null; rules?: string[] | null; log?: CallLog; queryText?: boolean } = {}) {
  const server = createMcpServer(
    { clientId: "c", connectionSetId: "set-1", scopes: ["mdbase:read"] },
    gateway,
    {} as OAuthService,
    undefined,
    {
      callLog: options.log,
      queryTextCollections: options.queryText ? new Set([COLLECTION]) : new Set(),
      jev: options.jev,
      indexList: options.indexes === undefined ? undefined : ({ peek: () => options.indexes } as unknown as IndexListCache),
      folderRules: options.rules === undefined ? undefined : ({ peek: () => options.rules } as unknown as FolderRulesCache)
    }
  );
  const [ct, st] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "t", version: "0" });
  await Promise.all([server.connect(st), client.connect(ct)]);
  const search = async (args: Record<string, unknown>) => {
    const r = await client.callTool({ name: "search_records", arguments: { connection_id: CONN, ...args } });
    const text = (r.content as Array<{ text: string }>)[0].text;
    return { isError: r.isError === true, json: JSON.parse(text), text };
  };
  return { client, search };
}

const jevWith = (relevance: Record<string, number>, extra: { index?: { path: string; probability: number }; anyIndexFits?: number } = {}): JevClient & { rank: ReturnType<typeof vi.fn> } => ({
  rank: vi.fn(async (input: JevRankInput) => ({
    relevance: new Map(input.candidates.map((c) => [c.path, relevance[c.path] ?? 0])),
    index: extra.index ?? null,
    anyIndexFits: extra.anyIndexFits ?? null
  }))
});

const manyRows = (n: number): Row[] =>
  Array.from({ length: n }, (_, i) => ({ path: `notas/n${String(i).padStart(2, "0")}.md`, title: `Nota ${i}`, description: `sobre reuniao ${i}`, body: `# Titulo\n\nTexto da reuniao ${i}.` }));

describe("search_records", () => {
  it("exports the constants", () => {
    expect(SEARCH_RESULT_MAX_CHARS).toBe(12_000);
    expect(JEV_MIN_RELEVANCE).toBe(0.3);
    expect(INDEX_MIN_PROBABILITY).toBe(0.7);
    expect(JEV_INDEX_BONUS).toBe(0.1);
  });

  it("is listed as read-only and mentioned in the server instructions", async () => {
    const { client } = await connect(fakeGateway([]));
    const tool = (await client.listTools()).tools.find((t) => t.name === "search_records");
    expect(tool?.annotations?.readOnlyHint).toBe(true);
    expect(SERVER_INSTRUCTIONS).toContain("To find records when you do not know their path, use search_records before browsing with query_records.");
  });

  it("happy path: 30 candidates, Jev order, up to limit, excerpt, folder rule", async () => {
    const rows = manyRows(30);
    const relevance: Record<string, number> = {};
    rows.forEach((r, i) => { relevance[r.path] = i === 29 ? 0.95 : i === 7 ? 0.9 : 0.5 - i / 100; });
    const jev = jevWith(relevance);
    const { search } = await connect(fakeGateway(rows), { jev, rules: ["AGENTS.md", "notas/AGENTS.md"] });
    const { json, isError } = await search({ query: "reuniao", limit: 3 });
    expect(isError).toBe(false);
    expect(json.ranking).toBe("jev");
    expect(json.results.map((r: { path: string }) => r.path)).toEqual(["notas/n29.md", "notas/n07.md", "notas/n00.md"]);
    expect(json.results[0]).toMatchObject({ title: "Nota 29", folder_rule: "notas/AGENTS.md", score: 0.95, heading: "Titulo" });
    expect(json.results[0].excerpt).toContain("reuniao 29");
    expect(typeof json.results[0].line_start).toBe("number");
    expect(json.candidates_considered).toBe(30);
    expect(json.candidates_total).toBe(30);
    expect(json.truncated).toBe(false);
    expect(json.start_here).toBeNull();
    expect(jev.rank).toHaveBeenCalledTimes(1);
    expect(jev.rank.mock.calls[0][0].candidates).toHaveLength(30);
    expect(jev.rank.mock.calls[0][0].candidates[0].excerpt).toContain("reuniao");
  });

  it("drops candidates below 0.3 when ranking by Jev", async () => {
    const rows = manyRows(4);
    const jev = jevWith({ "notas/n00.md": 0.8, "notas/n01.md": 0.3, "notas/n02.md": 0.29, "notas/n03.md": 0.1 });
    const { search } = await connect(fakeGateway(rows), { jev });
    const { json } = await search({ query: "reuniao" });
    expect(json.results.map((r: { path: string }) => r.path)).toEqual(["notas/n00.md", "notas/n01.md"]);
  });

  it("start_here appears above 0.7 and the index folder gets a bonus in the order only", async () => {
    const rows: Row[] = [
      { path: "a/x.md", title: "X", body: "reuniao" },
      { path: "b/y.md", title: "Y", body: "reuniao" },
      { path: "b/z.md", title: "Z", body: "reuniao" }
    ];
    const indexes = [{ path: "b/Hub_B.md", title: "Hub B", description: "sobre B" }];
    const jev = jevWith({ "a/x.md": 0.55, "b/y.md": 0.5, "b/z.md": 0.25 }, { index: { path: "b/Hub_B.md", probability: 0.9 }, anyIndexFits: 0.7 });
    const { search } = await connect(fakeGateway(rows), { jev, indexes });
    const { json } = await search({ query: "reuniao" });
    expect(json.start_here).toEqual({ path: "b/Hub_B.md", title: "Hub B", description: "sobre B" });
    // y: 0.5 + 0.1 beats x: 0.55; z stays out (raw 0.25 < 0.3 even with the bonus)
    expect(json.results.map((r: { path: string }) => r.path)).toEqual(["b/y.md", "a/x.md"]);
    expect(json.results[0].score).toBe(0.5);
  });

  it("start_here is null below 0.7 and without the index list", async () => {
    const rows = manyRows(2);
    const jev = jevWith({ "notas/n00.md": 0.8 }, { index: { path: "b/Hub_B.md", probability: 0.9 }, anyIndexFits: 0.69 });
    const { search } = await connect(fakeGateway(rows), { jev, indexes: [{ path: "b/Hub_B.md", title: "Hub B", description: null }] });
    expect((await search({ query: "reuniao" })).json.start_here).toBeNull();
    const cold = jevWith({ "notas/n00.md": 0.8 });
    const { search: s2 } = await connect(fakeGateway(rows), { jev: cold, indexes: null });
    expect((await s2({ query: "reuniao" })).json.start_here).toBeNull();
    expect(cold.rank.mock.calls[0][0].indexes).toEqual([]);
  });

  it("without Jev: pre_order, no warning, score is the preScore", async () => {
    const rows = manyRows(3);
    const { search } = await connect(fakeGateway(rows), { jev: null });
    const { json } = await search({ query: "reuniao" });
    expect(json.ranking).toBe("pre_order");
    expect(json.ranking_warning).toBeUndefined();
    expect(json.results).toHaveLength(3);
    expect(json.results[0].score).toBeGreaterThan(0);
  });

  it("Jev failure: pre_order with the error code, logged", async () => {
    const jev: JevClient = { rank: vi.fn(async () => { throw new JevError("jev_rate_limited", "429"); }) };
    const { entries, log } = fakeLog();
    const { search } = await connect(fakeGateway(manyRows(3)), { jev, log });
    const { json } = await search({ query: "reuniao" });
    expect(json.ranking).toBe("pre_order");
    expect(json.ranking_warning).toBe("jev_rate_limited");
    await vi.waitFor(() => expect(entries).toHaveLength(1));
    expect(entries[0]).toMatchObject({ tool: "search_records", ranking: "pre_order", errorCode: "jev_rate_limited", resultCount: 3 });
    const unknown: JevClient = { rank: vi.fn(async () => { throw new Error("boom"); }) };
    const { search: s2 } = await connect(fakeGateway(manyRows(3)), { jev: unknown });
    expect((await s2({ query: "reuniao" })).json.ranking_warning).toBe("jev_unavailable");
  });

  it("truncates from the end so the text actually sent stays within 12,000 characters", async () => {
    const big = "reuniao " + "palavra ".repeat(140);
    const rows = Array.from({ length: 20 }, (_, i) => ({ path: `n/n${String(i).padStart(2, "0")}.md`, title: `N ${i}`, body: big }));
    const { client } = await connect(fakeGateway(rows), { jev: null });
    const r = await client.callTool({ name: "search_records", arguments: { connection_id: CONN, query: "reuniao", limit: 20 } });
    const text = (r.content as Array<{ text: string }>)[0].text;
    const json = JSON.parse(text);
    expect(json.truncated).toBe(true);
    expect(json.results.length).toBeGreaterThan(0);
    expect(json.results.length).toBeLessThan(20);
    expect(text.length).toBeLessThanOrEqual(SEARCH_RESULT_MAX_CHARS);
    expect(text).toBe(JSON.stringify(json));
    expect((r.structuredContent as { results: unknown[] }).results).toHaveLength(json.results.length);
  });

  it("refuses a connection without query, reading nothing", async () => {
    const gateway = fakeGateway(manyRows(2), ["read"]);
    const { search } = await connect(gateway, { jev: jevWith({}) });
    const { isError, json } = await search({ query: "reuniao" });
    expect(isError).toBe(true);
    expect(json.error.code).toBe("insufficient_collection_access");
    expect(gateway.operation).not.toHaveBeenCalled();
  });

  it("no candidates: empty results and no Jev call", async () => {
    const jev = jevWith({});
    const { search } = await connect(fakeGateway([]), { jev });
    const { json } = await search({ query: "reuniao" });
    expect(json.results).toEqual([]);
    expect(json.candidates_total).toBe(0);
    expect(jev.rank).not.toHaveBeenCalled();
  });

  it("peeks the index list once and never awaits it", async () => {
    const order: string[] = [];
    const gateway = fakeGateway(manyRows(2), ["query"], (_op, input) => order.push(String(input.where).startsWith("file.path in") ? "bodies" : "terms"));
    const indexList = { peek: vi.fn(() => { order.push("peek-indexes"); return null; }) } as unknown as IndexListCache;
    const server = createMcpServer({ clientId: "c", connectionSetId: "set-1", scopes: ["mdbase:read"] }, gateway, {} as OAuthService, undefined, { indexList, jev: jevWith({}) });
    const [ct, st] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "t", version: "0" });
    await Promise.all([server.connect(st), client.connect(ct)]);
    await client.callTool({ name: "search_records", arguments: { connection_id: CONN, query: "reuniao" } });
    expect(order.indexOf("bodies")).toBeGreaterThan(-1);
    expect(order.filter((o) => o === "peek-indexes").length).toBe(1);
  });

  it("does not peek the index list without a Jev client", async () => {
    const gateway = fakeGateway(manyRows(2), ["query"]);
    const indexList = { peek: vi.fn(() => null) } as unknown as IndexListCache;
    const server = createMcpServer({ clientId: "c", connectionSetId: "set-1", scopes: ["mdbase:read"] }, gateway, {} as OAuthService, undefined, { indexList, jev: null });
    const [ct, st] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: "t", version: "0" });
    await Promise.all([server.connect(st), client.connect(ct)]);
    await client.callTool({ name: "search_records", arguments: { connection_id: CONN, query: "reuniao" } });
    expect(indexList.peek).not.toHaveBeenCalled();
  });

  it("title falls back to the file name without .md, in results and in the Jev state", async () => {
    const jev = jevWith({ "notas/sem-titulo.md": 0.9 });
    const { search } = await connect(fakeGateway([{ path: "notas/sem-titulo.md", body: "reuniao" }]), { jev });
    const { json } = await search({ query: "reuniao" });
    expect(json.results[0].title).toBe("sem-titulo");
    const sent = (jev.rank as unknown as { mock: { calls: Array<[{ candidates: Array<{ title: string | null }> }]> } }).mock.calls[0][0];
    expect(sent.candidates[0].title).toBe("sem-titulo");
  });

  it("mentions the line range base in the tool description", async () => {
    const { client } = await connect(fakeGateway([]));
    const tool = (await client.listTools()).tools.find((t) => t.name === "search_records");
    expect(tool?.description).toContain("line_start and line_end count lines in the record body as returned by read_record.");
  });

  it("warns when the bodies cannot be read, still returning results", async () => {
    const gateway = fakeGateway(manyRows(2), ["query"], (_op, input) => {
      if (String(input.where).startsWith("file.path in")) throw new Error("down");
    });
    const { search } = await connect(gateway, { jev: null });
    const { json, isError } = await search({ query: "reuniao" });
    expect(isError).toBe(false);
    expect(json.excerpt_warning).toBe("bodies_unavailable");
    expect(json.results).toHaveLength(2);
    expect(json.results[0].excerpt).toBeNull();
    const ok = await (await connect(fakeGateway(manyRows(2)), { jev: null })).search({ query: "reuniao" });
    expect(ok.json.excerpt_warning).toBeUndefined();
  });

  it("describes folder_rule null", async () => {
    const { client } = await connect(fakeGateway([]));
    const tool = (await client.listTools()).tools.find((t) => t.name === "search_records");
    expect(tool?.description).toContain("folder_rule is null when the folder has no rule or rules are not known yet.");
  });

  it("logs the query text only for allowed collections, never excerpts", async () => {
    for (const allowed of [true, false]) {
      const { entries, log } = fakeLog();
      const { search } = await connect(fakeGateway(manyRows(2)), { jev: null, log, queryText: allowed });
      await search({ query: "reuniao" });
      await vi.waitFor(() => expect(entries).toHaveLength(1));
      expect(entries[0].queryText).toBe(allowed ? "reuniao" : null);
      expect(entries[0].paths).toEqual(["notas/n00.md", "notas/n01.md"].sort());
      expect(JSON.stringify(entries[0])).not.toContain("Texto da reuniao");
    }
  });
});
