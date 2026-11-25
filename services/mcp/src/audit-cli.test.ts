import { describe, expect, it } from "vitest";
import { buildAuditQuery, buildCallLogQuery, runAuditQuery, runCallLogQuery } from "./audit-cli.js";
import { createDatabase } from "./db.js";
import { createPgWriteAudit } from "./write-audit.js";

describe("buildAuditQuery", () => {
  it("sem filtros usa só o limite padrão", () => {
    const query = buildAuditQuery({});

    expect(query.text).toContain("ORDER BY created_at DESC LIMIT $1");
    expect(query.values).toEqual([50]);
  });

  it("cliente e desde viram valores posicionais, sem interpolar no texto", () => {
    const query = buildAuditQuery({ cliente: "Enola", desde: "2026-11-13" });

    expect(query.values).toEqual(["Enola", "2026-11-13", 50]);
    expect(query.text).not.toContain("Enola");
    expect(query.text).not.toContain("2026-11-13");
  });

  it("ate com data simples inclui o dia inteiro", () => {
    const query = buildAuditQuery({ ate: "2026-11-13" });

    expect(query.values).toEqual(["2026-11-13", 50]);
    expect(query.text).toContain("created_at < ($1::date + interval '1 day')::timestamptz");
    expect(query.text).not.toContain("created_at <= $1");
  });

  it("ate com timestamp completo mantém comparação inclusiva", () => {
    const query = buildAuditQuery({ ate: "2026-11-13T12:00:00Z" });

    expect(query.values).toEqual(["2026-11-13T12:00:00Z", 50]);
    expect(query.text).toContain("created_at <= $1");
  });

  it("limite acima de 1000 é limitado a 1000", () => {
    const query = buildAuditQuery({ limite: 5000 });

    expect(query.values).toEqual([1000]);
  });
});

describe("runAuditQuery contra o banco", () => {
  it("filtra por cliente, nota e período e devolve TSV mais recente primeiro", async () => {
    const db = await createDatabase("memory");
    await db.query("INSERT INTO mcp_clients (id, client_name, redirect_uris) VALUES ('c1', 'Enola', '[]'), ('c2', 'Outra', '[]')");
    const audit = createPgWriteAudit(db);
    const entrada = (clientId: string, path: string | null, toPath: string | null = null) => ({
      clientId,
      connectionSetId: "00000000-0000-4000-8000-000000000001",
      connectionId: "00000000-0000-4000-8000-000000000002",
      tool: "update_record" as const,
      path,
      toPath,
      requestId: `req-${path}`
    });
    const idEnola = await audit.begin(entrada("c1", "00-conhecimento/backlog.md"));
    await audit.finish(idEnola, { ok: true, revision: "r2" });
    const idOutra = await audit.begin(entrada("c2", "00-conhecimento/backlog.md"));
    await audit.finish(idOutra, { ok: false, errorCode: "revision_conflict" });
    await audit.begin(entrada("c1", "02-sistemas/outro.md"));

    const tudo = await runAuditQuery(db, {});
    expect(tudo.split("\n")[0]).toBe("created_at\tclient_name\ttool\tpath\tto_path\toutcome\terror_code\trevision");
    expect(tudo.trim().split("\n")).toHaveLength(4);

    const enola = await runAuditQuery(db, { cliente: "Enola", nota: "backlog" });
    const linhas = enola.trim().split("\n").slice(1);
    expect(linhas).toHaveLength(1);
    expect(linhas[0]).toContain("Enola\tupdate_record\t00-conhecimento/backlog.md\t\tok\t\tr2");

    await db.query(
      "INSERT INTO mcp_write_audit (created_at, client_id, connection_set_id, connection_id, tool, path, request_id, outcome) VALUES ('2026-11-13T15:00:00Z', 'c1', 's', 'k', 'create_record', 'dia.md', 'req-dia', 'ok')"
    );
    const diaInteiro = await runAuditQuery(db, { ate: "2026-11-13" });
    expect(diaInteiro).toContain("dia.md");
    const ateMeiaNoite = await runAuditQuery(db, { ate: "2026-11-13T00:00:00Z" });
    expect(ateMeiaNoite).not.toContain("dia.md");

    const hoje = await runAuditQuery(db, { desde: "2000-01-01", limite: 1 });
    expect(hoje.trim().split("\n")).toHaveLength(2);
    await db.end();
  });
});

describe("buildCallLogQuery", () => {
  it("sem filtros usa só o limite padrão", () => {
    const query = buildCallLogQuery({});

    expect(query.text).toContain("FROM mcp_call_log");
    expect(query.text).toContain("ORDER BY created_at DESC LIMIT $1");
    expect(query.values).toEqual([50]);
  });

  it("cliente, coleção, ferramenta e desde viram valores posicionais", () => {
    const query = buildCallLogQuery({ cliente: "Enola", colecao: "col-1", ferramenta: "query_records", desde: "2026-11-13" });

    expect(query.values).toEqual(["Enola", "col-1", "query_records", "2026-11-13", 50]);
    expect(query.text).toContain("client_name = $1");
    expect(query.text).toContain("collection_id = $2");
    expect(query.text).toContain("tool = $3");
    expect(query.text).not.toContain("Enola");
  });

  it("ate com data simples inclui o dia inteiro; com timestamp é inclusivo", () => {
    expect(buildCallLogQuery({ ate: "2026-11-13" }).text).toContain("created_at < ($1::date + interval '1 day')::timestamptz");
    expect(buildCallLogQuery({ ate: "2026-11-13T12:00:00Z" }).text).toContain("created_at <= $1");
  });

  it("limite acima de 1000 é limitado a 1000", () => {
    expect(buildCallLogQuery({ limite: 5000 }).values).toEqual([1000]);
  });

  it("query_text e paths só aparecem com detalhes", () => {
    expect(buildCallLogQuery({}).text).not.toMatch(/query_text|paths/);
    expect(buildCallLogQuery({}, true).text).toMatch(/paths/);
    expect(buildCallLogQuery({}, true).text).toMatch(/query_text/);
  });
});

describe("runCallLogQuery", () => {
  it("devolve TSV do registro filtrado por ferramenta", async () => {
    const db = await createDatabase("memory");
    await db.query(
      `INSERT INTO mcp_call_log (client_id, client_name, connection_set_id, tool, duration_ms, outcome, result_count, paths, query_text)
       VALUES ('c', 'Enola', 's', 'query_records', 42, 'ok', 3, ARRAY['a.md'], 'backlog'),
              ('c', 'Enola', 's', 'read_record', 7, 'ok', 1, ARRAY['b.md'], NULL)`
    );

    const sem = await runCallLogQuery(db, { ferramenta: "query_records" });
    expect(sem.split("\n")[0]).toBe("created_at\tclient_name\ttool\tduration_ms\tresult_count\toutcome\terror_code\tranking");
    expect(sem).toContain("query_records");
    expect(sem).not.toContain("read_record");
    expect(sem).not.toContain("backlog");

    const com = await runCallLogQuery(db, { ferramenta: "query_records" }, true);
    expect(com).toContain("backlog");
    expect(com).toContain("a.md");
    await db.end();
  });
});
