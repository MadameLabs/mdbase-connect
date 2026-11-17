#!/usr/bin/env node
/** LOCAL TEST: `mdbase tasks` through the daemon, on the local "mdbase next" stack.
 * Nothing here is LAB, staging or production, and no installed daemon, account or
 * managed artifact is used or changed.
 *
 *   node runtime/packages/sdk/scripts/madamelabs-tasks-local-e2e.mjs
 *
 * Options: `--rebuild` rebuilds the cargo artifacts; `--keep` keeps the run's
 * scratch directory and logs. Environment: `MDBASE_E2E_CARGO` (`cargo`, or `rcargo`
 * on the build VM), `MDBASE_TASKS_BIBLIOTECA` (the BIBLIOTECA folder; default the
 * Arthur-PC path). Needs Docker (one disposable PostgreSQL container, removed at the
 * end) and, for the desktop's keychain, `dbus-daemon`, `gnome-keyring-daemon`,
 * `gdbus` and `secret-tool`.
 *
 * The stack, the desktop and the Private collection come from
 * app-private-device-local-e2e.mjs (its step 1, without the password setup and
 * without the app). The collection is the BIBLIOTECA's real `task` type and its
 * `tasknotes.task` contract, with a `pessoa` type for `--responsavel`. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, createWriteStream, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, request as httpRequest } from "node:http";
import { createRequire } from "node:module";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startHostedPostgres } from "../../../../scripts/lib/hosted-postgres-container.mjs";
import { availableTcpPort, delay, poll } from "../../../../scripts/lib/test-runtime.mjs";
import { bundleLocalServer, checkout, e2eRoot, ensureDesktop, ensureStack, ensureTools } from "./app-private-device-local-stack.mjs";

const flags = new Set(process.argv.slice(2));
for (const flag of flags) if (!["--rebuild", "--keep"].includes(flag)) throw Error(`unknown option ${flag}`);
const rebuild = flags.has("--rebuild"), keep = flags.has("--keep");
const step = name => console.log(`ok - ${name}`);
const section = name => console.log(`\n== ${name}`);
const biblioteca = process.env.MDBASE_TASKS_BIBLIOTECA ?? "/home/arthur/OFICINA_Madame-Labs/BIBLIOTECA";
const taskType = join(biblioteca, "_config/mdbase/types/task.md");
const taskContract = join(biblioteca, "_config/mdbase/contracts/tasknotes.task.md");

// ---------------------------------------------------------------- build inputs
section("build inputs");
const server = await bundleLocalServer();
const stack = ensureStack(server.keys);
const tools = ensureTools(rebuild);
const desktopBinary = ensureDesktop(stack, tools.verifier, rebuild);
const connectOrigin = stack.config.connectOrigin, logOrigin = stack.config.logOrigin;
step(`local stack ${connectOrigin} (Connect), ${logOrigin} (log service)`);

// --------------------------------------------------------------------- the run
const run = join(e2eRoot, "runs", `tasks-${new Date().toISOString().replace(/[:.]/g, "-")}-${process.pid}`);
mkdirSync(run, { recursive: true });
// Short: the daemon's Unix sockets live under its state directory.
const scratch = await mkdtemp(join(tmpdir(), "mdpt-"));
const cleanup = [];
const children = [];
const nonce = randomBytes(6).toString("hex");
const taskTitle = "Prova next";

function child(name, command, args, env, options = {}) {
  const log = createWriteStream(join(run, `${name}.log`), { flags: "a" });
  const process_ = spawn(command, args, { env, stdio: ["ignore", "pipe", "pipe"], ...options });
  process_.stdout.pipe(log, { end: false }); process_.stderr.pipe(log, { end: false });
  const entry = { name, process: process_, exited: new Promise(done => process_.once("exit", (code, signal) => done(code ?? signal))) };
  children.push(entry);
  return entry;
}
async function stop(entry, signal = "SIGTERM") {
  if (entry.process.exitCode === null && entry.process.signalCode === null) entry.process.kill(signal);
  await Promise.race([entry.exited, delay(10_000).then(() => entry.process.kill("SIGKILL"))]);
}
async function portFree(origin) {
  const port = Number(new URL(origin).port);
  await new Promise((free, busy) => {
    const probe = createServer();
    probe.once("error", () => busy(Error(`${origin} is in use; the local stack needs its pinned port (stop the other run, or delete ${stack.directory} for a new identity and rebuild)`)));
    probe.listen(port, "127.0.0.1", () => probe.close(free));
  });
  return port;
}

/** A recording loopback front for one pinned origin (the stack's origins are
 * pinned in the signed trust asset). WebSocket frames are unmasked and kept per
 * direction; the log service also gets the CORS headers the deployed Worker adds. */
function front({ port, target, cors }) {
  const exchanges = [], streams = [], sockets = new Set();
  const allow = { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, POST, PUT",
    "access-control-allow-headers": "authorization, content-type, x-mdbase-nonce, x-mdbase-sig, range, x-amz-checksum-sha256",
    "access-control-expose-headers": "content-length, content-range, etag, x-amz-checksum-sha256, x-mdbase-next-nonce" };
  const listener = createServer((request, response) => {
    const parts = [];
    request.on("data", part => parts.push(part));
    request.on("end", () => {
      const exchange = { method: request.method, path: request.url, headers: request.headers, body: Buffer.concat(parts), status: null, reply: null };
      if (cors && request.method === "OPTIONS") { response.writeHead(204, { ...allow, "access-control-max-age": "7200" }); response.end(); return; }
      exchanges.push(exchange);
      const headers = { ...request.headers }; delete headers["accept-encoding"];
      const upstream = httpRequest({ host: "127.0.0.1", port: target, method: request.method, path: request.url, headers }, reply => {
        const received = [];
        response.writeHead(reply.statusCode, { ...reply.headers, ...(cors ? allow : {}) });
        reply.on("data", part => { received.push(part); response.write(part); });
        reply.on("end", () => { exchange.reply = Buffer.concat(received); exchange.status = reply.statusCode; response.end(); });
        reply.on("error", () => response.destroy());
      });
      upstream.on("error", () => { exchange.status = 502; exchange.reply = Buffer.alloc(0); if (!response.headersSent) response.writeHead(502); response.end(); });
      upstream.end(exchange.body);
    });
  });
  const frames = (sink, skipHead) => {
    let pending = Buffer.alloc(0), head = skipHead;
    return part => {
      pending = Buffer.concat([pending, part]);
      if (head) { const end = pending.indexOf("\r\n\r\n"); if (end < 0) return; pending = pending.subarray(end + 4); head = false; }
      for (;;) {
        if (pending.length < 2) return;
        let length = pending[1] & 0x7f, offset = 2; const masked = (pending[1] & 0x80) !== 0;
        if (length === 126) { if (pending.length < 4) return; length = pending.readUInt16BE(2); offset = 4; }
        else if (length === 127) { if (pending.length < 10) return; length = Number(pending.readBigUInt64BE(2)); offset = 10; }
        const key = masked ? pending.subarray(offset, offset + 4) : null; if (masked) offset += 4;
        if (pending.length < offset + length) return;
        const payload = Buffer.from(pending.subarray(offset, offset + length));
        if (key) for (let i = 0; i < payload.length; i++) payload[i] ^= key[i & 3];
        if ((pending[0] & 0x0f) <= 2) sink(payload, (pending[0] & 0x40) !== 0);
        pending = pending.subarray(offset + length);
      }
    };
  };
  listener.on("upgrade", (request, socket, first) => {
    const stream = { path: request.url, origin: request.headers.origin ?? null, sent: [], received: [], compressed: false };
    streams.push(stream);
    const upstream = connect(target, "127.0.0.1", () => {
      const lines = [`${request.method} ${request.url} HTTP/1.1`];
      for (let i = 0; i < request.rawHeaders.length; i += 2) if (request.rawHeaders[i].toLowerCase() !== "sec-websocket-extensions") lines.push(`${request.rawHeaders[i]}: ${request.rawHeaders[i + 1]}`);
      upstream.write(lines.join("\r\n") + "\r\n\r\n");
      const sent = frames((payload, rsv1) => { stream.sent.push(payload); stream.compressed ||= rsv1; }, false);
      const received = frames((payload, rsv1) => { stream.received.push(payload); stream.compressed ||= rsv1; }, true);
      if (first.length) { sent(first); upstream.write(first); }
      socket.on("data", part => { sent(part); upstream.write(part); });
      upstream.on("data", part => { received(part); socket.write(part); });
      socket.on("close", () => upstream.destroy()); upstream.on("close", () => socket.destroy());
    });
    socket.on("error", () => upstream.destroy()); upstream.on("error", () => socket.destroy());
  });
  listener.on("connection", socket => { sockets.add(socket); socket.once("close", () => sockets.delete(socket)); });
  return { exchanges, streams, listen: () => new Promise((ready, failed) => { listener.once("error", failed); listener.listen(port, "127.0.0.1", ready); }),
    close: () => new Promise(done => { listener.close(() => done()); for (const socket of sockets) socket.destroy(); }) };
}

let pg, postgres, connect_ = null, log = null;
let failed = false;
try {
  // ------------------------------------------------------------ the local stack
  section("local stack");
  const connectPort = await portFree(connectOrigin), logPort = await portFree(logOrigin);
  postgres = await startHostedPostgres("madamelabs-tasks-e2e");
  cleanup.push(() => postgres.stop());
  await postgres.createDatabase("madamelabs_tasks_e2e_test");
  const databaseUrl = postgres.databaseUrl("madamelabs_tasks_e2e_test");
  step(`disposable PostgreSQL container ${postgres.container}`);

  const logInner = await availableTcpPort(), connectInner = await availableTcpPort();
  child("log-service", tools.log, ["--listen", `127.0.0.1:${logInner}`, "--backend", "mem", "--public-base", logOrigin], { PATH: process.env.PATH, ...stack.logEnv });
  log = front({ port: logPort, target: logInner, cors: true }); await log.listen(); cleanup.push(() => log.close());
  await poll(async () => (await fetch(`${logOrigin}/health`)).ok, "the log service never answered", 100, 100);
  step("native log service (memory backend) behind its recording front");

  // Outbound cloud-copy settings the listing route's mount requires; nothing listens there.
  const token = () => randomBytes(24).toString("hex");
  const serverEnv = { PATH: process.env.PATH, ...stack.serverEnv, PORT: String(connectInner), DATABASE_URL: databaseUrl,
    MDBASE_NEXT_CLOUD_COPY_BOOTSTRAP: "1", MDBASE_NEXT_HOSTED_SERVICE_URL: "http://127.0.0.1:9", MDBASE_NEXT_ESCROW_SERVICE_URL: "http://127.0.0.1:9",
    MDBASE_NEXT_HOSTED_SERVICE_TOKEN: token(), MDBASE_NEXT_ESCROW_SERVICE_TOKEN: token(), MDBASE_NEXT_HOSTED_INTERNAL_TOKEN: token(), MDBASE_NEXT_ESCROW_INTERNAL_TOKEN: token() };
  // The portal build is not needed here; the server's approval-page rule still wants a directory.
  const portalDist = join(run, "portal"); mkdirSync(portalDist, { recursive: true });
  writeFileSync(join(portalDist, "index.html"), '<!doctype html><meta charset="utf-8"><title>Portal stand-in</title><p>Portal stand-in: the test performs the portal\'s requests.</p>');
  serverEnv.PORTAL_DIST = portalDist;
  connect_ = front({ port: connectPort, target: connectInner, cors: false }); await connect_.listen(); cleanup.push(() => connect_.close());
  const connectServer = child("connect", process.execPath, [server.entry], serverEnv);
  const exited = connectServer.exited.then(code => { throw Error(`the Connect server exited (${code}); see ${join(run, "connect.log")}`); });
  exited.catch(() => {});
  await Promise.race([exited, poll(async () => (await fetch(`${connectOrigin}/health`)).ok, "the Connect server never answered", 300, 100)]);
  cleanup.push(() => stop(connectServer));
  step("Connect server (own entry, PostgreSQL, private bootstrap)");

  const require_ = createRequire(join(checkout, "services/server/package.json"));
  pg = new (require_("pg").Pool)({ connectionString: databaseUrl, max: 2 });
  cleanup.push(() => pg.end());
  const rows = async (text, values = []) => (await pg.query(text, values)).rows;

  // ---------------------------------------------------- the account (dev sign-in)
  const session = await fetch(`${connectOrigin}/v1/dev/session`, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ name: "Tasks e2e", email: `madamelabs-tasks-${nonce}@example.test` }) });
  const cookie = session.headers.get("set-cookie")?.split(";")[0];
  if (!session.ok || !cookie) throw Error(`development sign-in failed (${session.status})`);
  // The development route leaves the account on the legacy backend; every real sign-up runs this function for a new account.
  const accountId = (await session.json()).user.id, writer = await pg.connect();
  try { await writer.query("BEGIN"); await server.keys.provisionNewAccountBackend(writer, accountId, "next"); await writer.query("COMMIT"); }
  catch (error) { await writer.query("ROLLBACK").catch(() => {}); throw error; } finally { writer.release(); }
  /** The portal's own requests, on the real routes, as the signed-in person. */
  const portal = async (method, path, body) => {
    const reply = await fetch(`${connectOrigin}${path}`, { method, headers: { cookie, origin: connectOrigin, ...(body === undefined ? {} : { "content-type": "application/json" }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
    const text = await reply.text(); let value = null; try { value = JSON.parse(text); } catch { /* not JSON */ }
    return { status: reply.status, value, text };
  };

  // ------------------------------------------------------- the desktop and collection
  section("1. the desktop creates a Private collection (the BIBLIOTECA's task type)");
  const state = join(scratch, "s"), folder = join(scratch, "biblioteca"), home = join(scratch, "h");
  await mkdir(home, { recursive: true });
  // Nothing of the person's desktop session is inherited: its own HOME, XDG directories and session bus, and on that bus its own keychain.
  const bus = `unix:path=${join(scratch, "bus")}`;
  const desktopEnv = { PATH: process.env.PATH, HOME: home, XDG_CONFIG_HOME: join(home, "config"), XDG_DATA_HOME: join(home, "data"), XDG_STATE_HOME: join(home, "state"),
    XDG_RUNTIME_DIR: join(home, "run"), DBUS_SESSION_BUS_ADDRESS: bus, LANG: "C.UTF-8", MDBASE_CONNECT_HOME: join(scratch, "no-connect"), MDBASE_HOME: state };
  for (const directory of [desktopEnv.XDG_CONFIG_HOME, desktopEnv.XDG_DATA_HOME, desktopEnv.XDG_STATE_HOME, desktopEnv.XDG_RUNTIME_DIR]) await mkdir(directory, { recursive: true, mode: 0o700 });
  const tool = (command, args, input) => new Promise((done, refused) => {
    const process_ = spawn(command, args, { env: desktopEnv, stdio: ["pipe", "pipe", "pipe"] });
    let out = "", err = ""; process_.stdout.on("data", part => { out += part; }); process_.stderr.on("data", part => { err += part; });
    process_.once("error", refused);
    process_.once("exit", code => code === 0 ? done(out) : refused(Error(`${command} ${args.join(" ")} failed (${code}): ${err}`)));
    process_.stdin.end(input ?? "");
  });
  child("desktop-bus", "dbus-daemon", ["--session", "--nofork", `--address=${bus}`], desktopEnv);
  await poll(() => existsSync(join(scratch, "bus")), "the private session bus never started", 100, 50);
  const keyring = child("desktop-keyring", "gnome-keyring-daemon", ["--foreground", "--unlock", "--components=secrets"], desktopEnv, { stdio: ["pipe", "pipe", "pipe"] });
  keyring.process.stdin.end(randomBytes(32).toString("hex"));
  await tool("gdbus", ["wait", "--session", "--timeout", "14", "org.freedesktop.secrets"]);
  // A storage probe only (never a credential), as with-release-keyring does before any pairing.
  await tool("secret-tool", ["store", "--label=[test] madamelabs tasks e2e keychain probe", "service", "dev.mdbase.e2e.keychain-probe"], "keychain-probe");
  assert.equal(await tool("secret-tool", ["lookup", "service", "dev.mdbase.e2e.keychain-probe"]), "keychain-probe");
  await tool("secret-tool", ["clear", "service", "dev.mdbase.e2e.keychain-probe"]);
  step("throwaway OS keychain on a private session bus");
  /** The real CLI as its own process; a secret goes in on stdin, never as an argument. */
  const mdbase = (args, input) => new Promise((done, refused) => {
    const cli = spawn(desktopBinary, ["--state-dir", state, "--json", ...args], { env: desktopEnv, cwd: scratch, stdio: ["pipe", "pipe", "pipe"] });
    let out = "", err = ""; cli.stdout.on("data", part => { out += part; }); cli.stderr.on("data", part => { err += part; });
    cli.once("error", refused);
    cli.once("exit", code => {
      if (code === 0) { try { done(out.trim() ? JSON.parse(out) : null); } catch { refused(Error(`mdbase ${args.join(" ")}: unreadable output:\n${out}`)); } return; }
      const error = Error(`mdbase ${args.join(" ")} failed (${code}):\n${out}${err}`); error.stdout = out; refused(error);
    });
    cli.stdin.end(input === undefined ? "" : `${input}\n`);
  });
  // The collection: the BIBLIOTECA's task type and contract, a person type with one person (--responsavel), and a plain tasks config.
  await mkdir(join(folder, "_types"), { recursive: true }); await mkdir(join(folder, "_contracts"), { recursive: true });
  await mkdir(join(folder, "_config"), { recursive: true }); await mkdir(join(folder, "pessoas"), { recursive: true });
  await mkdir(join(folder, "06-tarefas", "Tasks"), { recursive: true });
  await writeFile(join(folder, "mdbase.yaml"), 'spec_version: "0.3.0"\nname: Prova next\n');
  await writeFile(join(folder, "_types", "task.md"), await readFile(taskType));
  await writeFile(join(folder, "_contracts", "tasknotes.task.md"), await readFile(taskContract));
  await writeFile(join(folder, "_types", "pessoa.md"), "---\nkind: mdbase.type\nname: pessoa\nmatch:\n  path_glob: \"pessoas/**/*.md\"\nschema:\n  dialect: json-schema-2020-12\n  value:\n    type: object\n---\n");
  await writeFile(join(folder, "pessoas", "Arthur.md"), "---\ntype: pessoa\ntitle: Arthur\n---\n");
  await writeFile(join(folder, "_config", "mdbase-tasks.json"), "{}\n");
  const daemon = child("desktop-daemon", desktopBinary, ["--state-dir", state, "daemon", "run"], desktopEnv, { cwd: scratch });
  cleanup.push(async () => { await mdbase(["daemon", "stop"]).catch(() => {}); await stop(daemon); });
  const daemonGone = daemon.exited.then(code => { throw Error(`the desktop daemon exited (${code}); see ${join(run, "desktop-daemon.log")}`); });
  daemonGone.catch(() => {});
  await Promise.race([daemonGone, poll(async () => (await mdbase(["daemon", "status"])).readiness.ready, "the desktop daemon never became ready", 200, 100)]);
  const signIn = await mdbase(["account", "sign-in", "--server", connectOrigin]);
  const desktopPairing = new URL(signIn.verification_uri).pathname.split("/").pop();
  const approvedDesktop = await portal("POST", `/v1/pairing-requests/${desktopPairing}/approve`, {});
  if (approvedDesktop.status !== 200) throw Error(`the desktop's sign-in was not approved (${approvedDesktop.status}): ${approvedDesktop.text}`);
  await poll(async () => {
    const { account } = await mdbase(["status"]);
    if (account.last_error && !account.pairing) throw Error(`desktop sign-in failed: ${JSON.stringify(account.last_error)}`);
    return account.signed_in && !account.pairing;
  }, "the desktop never finished signing in", 300, 100);
  step("desktop daemon (real mdbase, isolated profile) signed in to the local Connect");
  const added = await mdbase(["collection", "add", folder, "--name", "Prova next"]);
  await poll(async () => (await mdbase(["collection", "list"])).some(c => c.id === added.id && c.state === "ready"), "the collection never became ready", 300, 100);
  const collectionId = added.id;
  const enabled = await mdbase(["sync", "enable", collectionId, "--mode", "private"]);
  console.log(`   sync enable: ${JSON.stringify(enabled)}`);
  await poll(async () => (await rows("SELECT sync FROM next_collections WHERE collection_id=$1", [collectionId]))[0]?.sync === "private", "Connect never recorded the Private collection", 300, 100);
  const recorded = (await rows("SELECT * FROM next_collections WHERE collection_id=$1", [collectionId]))[0];
  console.log(`   next_collections row: ${JSON.stringify(recorded)}`);
  step(`Private collection ${collectionId} with the task type, contract and person ready`);

  // ------------------------------------------------------------- 2. the tasks
  section("2. mdbase tasks through the daemon (-c, --json)");
  const expectOk = (envelope, what) => { assert.equal(envelope?.ok, true, `${what}: ${JSON.stringify(envelope)}`); assert.ok(!("write_held" in (envelope ?? {})), `${what}: write_held`); return envelope; };
  const taskCmd = (...args) => mdbase(["tasks", ...args, "-c", collectionId]);

  const created = expectOk(await taskCmd("create", taskTitle, "--responsavel", "Arthur"), "tasks create");
  const path = created.result.path;
  assert.match(path, /\.md$/, `the created path is a note: ${path}`);
  assert.ok(readFileSync(join(folder, path), "utf8").includes(taskTitle), "the task note was written to the collection folder");
  step(`tasks create "${taskTitle}" --responsavel Arthur: ${path}`);

  const listed = expectOk(await taskCmd("list"), "tasks list");
  assert.ok(JSON.stringify(listed.result).includes(path), `tasks list does not contain ${path}: ${JSON.stringify(listed.result)}`);
  step("tasks list contains the task");

  const started = expectOk(await taskCmd("start", path, "--responsavel", "Arthur"), "tasks start");
  assert.ok(readFileSync(join(folder, path), "utf8").includes("em-andamento"), "the start status reached the file");
  step(`tasks start ${path} --responsavel Arthur: ${JSON.stringify(started.result?.status ?? "ok")}`);

  const timers = expectOk(await taskCmd("timer", "status"), "tasks timer status");
  console.log(`   timer status: ${JSON.stringify(timers.result)}`);
  assert.equal(Array.isArray(timers.result) ? timers.result.length : null, 1, `one open timer expected: ${JSON.stringify(timers.result)}`);
  step("tasks timer status: one open entry");

  const stopped = expectOk(await taskCmd("stop", path, "--responsavel", "Arthur", "--status", "cancelado"), "tasks stop");
  assert.equal(stopped.result.closed?.length, 1, `one closed entry expected: ${JSON.stringify(stopped.result)}`);
  assert.equal(stopped.result.warning, undefined, `no warning expected: ${JSON.stringify(stopped.result)}`);
  step(`tasks stop ${path} --status cancelado: closed one entry, no write_held`);

  // The write is published: the daemon's own read and the note on disk both say so.
  const shown = expectOk(await taskCmd("show", path), "tasks show");
  assert.equal(shown.result.frontmatter.status, "cancelado", JSON.stringify(shown.result.frontmatter));
  assert.equal(shown.result.frontmatter.timeEntries.length, 1, JSON.stringify(shown.result.frontmatter.timeEntries));
  assert.ok(typeof shown.result.frontmatter.timeEntries[0].endTime === "string", "the time entry is closed");
  assert.match(readFileSync(join(folder, path), "utf8"), /status: cancelado/, "the note on disk says cancelado");
  step("the stop is published: tasks show and the note on disk both say cancelado, with the time entry closed");

  console.log("\nPASS: mdbase tasks through the daemon, on the local next stack");
} catch (error) {
  failed = true;
  console.error(`\nFAILED: ${error?.stack ?? error}`);
  console.error(`logs: ${run}`);
} finally {
  if (connect_) writeFileSync(join(run, "connect-exchanges.jsonl"), connect_.exchanges.map(exchange => JSON.stringify({ method: exchange.method, path: exchange.path, status: exchange.status,
    origin: exchange.headers.origin ?? null, reply: exchange.reply?.toString("utf8").slice(0, 4000) ?? null })).join("\n") + "\n");
  // Bounded: a step that hangs must not keep the container or a child alive.
  for (const close of cleanup.reverse()) await Promise.race([Promise.resolve().then(close).catch(() => {}), delay(15_000)]);
  for (const entry of children) await stop(entry).catch(() => {});
  if (!keep) await rm(scratch, { recursive: true, force: true });
  else console.log(`kept: ${scratch}`);
}
process.exit(failed ? 1 : 0);
