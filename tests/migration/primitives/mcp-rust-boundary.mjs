/** Probe authenticated MCP replay isolation using only two synthetic disposable users. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { BASELINE, WORKSPACE_ROOT, loadOracle } from "../oracle.mjs";

const oracle = await loadOracle(BASELINE);
const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-mcp-boundary-"));
let child;
let exited;
try {
  await fs.writeFile(
    path.join(root, ".litradar-e2e-root"),
    "litradar-full-stack-e2e-v1\n",
  );
  await fs.writeFile(path.join(root, "secret.key"), Buffer.alloc(32, 42), {
    mode: 0o600,
  });
  const seeded = spawnSync(
    path.join(oracle.directory, oracle.manifest.binaries.fullStackFixture),
    ["--project-root", root],
    { cwd: root, timeout: 30_000, encoding: "utf8", maxBuffer: 1024 * 1024 },
  );
  assert.ifError(seeded.error);
  assert.equal(seeded.status, 0, seeded.stderr);
  const reserved = net.createServer();
  await new Promise((resolve, reject) => {
    reserved.once("error", reject);
    reserved.listen(0, "127.0.0.1", resolve);
  });
  const port = reserved.address().port;
  await new Promise((resolve) => reserved.close(resolve));
  const base = `http://127.0.0.1:${port}`;
  child = spawn(
    path.join(oracle.directory, oracle.manifest.binaries.application),
    [
      "serve",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--project-root",
      root,
      "--secret-key-file",
      path.join(root, "secret.key"),
      "--scheduler-interval-seconds",
      "3600",
      "--development",
    ],
    { cwd: root, stdio: "ignore", shell: false },
  );
  exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", resolve);
  });
  const deadline = Date.now() + 30_000;
  let ready = false;
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null);
    try {
      ready = (
        await fetch(`${base}/health/ready`, {
          signal: AbortSignal.timeout(500),
        })
      ).ok;
    } catch {}
    if (ready) break;
    await delay(100);
  }
  assert(ready);
  /**
   * Authenticate a public synthetic fixture identity.
   * @param {string} username - Fixture username.
   * @param {string} password - Public fixture password.
   * @returns {Promise<string>} Ephemeral cookie, never persisted.
   */
  async function login(username, password) {
    const response = await fetch(`${base}/api/auth/login`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ username, password }),
      signal: AbortSignal.timeout(10_000),
    });
    assert.equal(response.status, 200);
    await response.arrayBuffer();
    return response.headers
      .getSetCookie()
      .find((cookie) => cookie.startsWith("litradar_session="))
      .split(";")[0];
  }
  const owner = await login("fullstack_member", "FullStackMember!2026");
  const other = await login("fullstack_admin", "FullStackAdmin!2026");
  /**
   * Send one raw MCP message and fully consume its bounded response.
   * @param {string} cookie - Current request credential.
   * @param {object} message - JSON-RPC message.
   * @param {string} [session] - MCP session handle.
   * @returns {Promise<object>} Status, body, and ephemeral session.
   */
  async function post(cookie, message, session) {
    const headers = {
      cookie,
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
    };
    if (session)
      Object.assign(headers, {
        "mcp-session-id": session,
        "mcp-protocol-version": "2025-06-18",
      });
    const response = await fetch(`${base}/mcp`, {
      method: "POST",
      headers,
      body: JSON.stringify(message),
      signal: AbortSignal.timeout(10_000),
    });
    return {
      status: response.status,
      body: await response.text(),
      session: response.headers.get("mcp-session-id"),
    };
  }
  const initial = await post(owner, {
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "boundary-fixture", version: "1" },
    },
  });
  assert.equal(initial.status, 200);
  assert(initial.session);
  assert.equal(
    (
      await post(
        owner,
        { jsonrpc: "2.0", method: "notifications/initialized" },
        initial.session,
      )
    ).status,
    202,
  );
  const httpBoundary = [];
  const rawBodies = [
    '{"jsonrpc":"2.0","id":1.5,"method":"ping"}',
    '{"jsonrpc":"2.0","id":9223372036854775808,"method":"ping"}',
    '{"jsonrpc":"2.0","id":null,"method":"ping"}',
    '{"jsonrpc":"2.0","id":1}',
    '{"jsonrpc":"2.0","id":1,"method":null}',
    '{"jsonrpc":"2.0","error":{"code":-32600,"message":"fixture"}}',
    '{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"fixture"}}',
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}',
    '{"jsonrpc":"2.0","id":1,"method":"ping"} {}',
    "\n{",
    "  \n  ",
    '{"x":}',
    '{"x" 1}',
    '{"x":1,}',
    "[1,]",
    '{"x":"\\uD800"}',
    '{"x":"\\q"}',
    '{"x":01}',
    '{"x":1e999}',
  ];
  for (const specimen of [
    { name: "get-missing-accept", method: "GET" },
    {
      name: "get-no-session",
      method: "GET",
      headers: { accept: "text/event-stream" },
    },
    {
      name: "get-unknown-before-version",
      method: "GET",
      headers: {
        accept: "text/event-stream",
        "mcp-session-id": "unknown",
        "mcp-protocol-version": "bad",
      },
    },
    {
      name: "delete-unknown",
      method: "DELETE",
      headers: { "mcp-session-id": "unknown" },
    },
    { name: "post-accept-before-content-type", method: "POST", body: "{}" },
    {
      name: "post-bad-content-type",
      method: "POST",
      headers: { accept: "application/json, text/event-stream" },
      body: "{}",
    },
    ...[
      "{}",
      "[]",
      "{",
      "",
      "null",
      '{"jsonrpc":"2.0","id":99,"method":"not/a/method"}',
      '{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{}}',
      '{"jsonrpc":"2.0","id":99,"method":"ping","params":{}}',
    ].map((body, index) => ({
      name: `post-body-${index}`,
      method: "POST",
      headers: {
        accept: "application/json, text/event-stream",
        "content-type": "application/json",
      },
      body,
    })),
    ...rawBodies.map((body, index) => ({
      name: `raw-session-${index}`,
      method: "POST",
      useSession: true,
      headers: {
        accept: "application/json, text/event-stream",
        "content-type": "application/json",
      },
      body,
    })),
  ]) {
    const observed = await fetch(`${base}/mcp`, {
      method: specimen.method,
      headers: {
        cookie: owner,
        ...specimen.headers,
        ...(specimen.useSession
          ? {
              "mcp-session-id": initial.session,
              "mcp-protocol-version": "2025-06-18",
            }
          : {}),
      },
      body: specimen.body,
      signal: AbortSignal.timeout(5_000),
    });
    httpBoundary.push({
      ...specimen,
      status: observed.status,
      responseBody: await observed.text(),
      contentType: observed.headers.get("content-type"),
    });
  }
  const folders = await post(
    owner,
    {
      jsonrpc: "2.0",
      id: 10,
      method: "tools/call",
      params: { name: "list_folders", arguments: {} },
    },
    initial.session,
  );
  assert.equal(folders.status, 200);
  assert(folders.body.includes("Reading"));
  const ids = [...folders.body.matchAll(/^id:\s*(\S+)/gm)].map(
    (match) => match[1],
  );
  const lastEventId = ids.at(-1);
  assert(lastEventId);
  const independent = await fetch(`${base}/api/favorites/folders`, {
    headers: { cookie: other },
    signal: AbortSignal.timeout(5_000),
  });
  const independentBody = await independent.text();
  assert.equal(independent.status, 200);
  assert(!independentBody.includes("Reading"));
  const response = await fetch(`${base}/mcp`, {
    headers: {
      cookie: other,
      accept: "text/event-stream",
      "mcp-session-id": initial.session,
      "mcp-protocol-version": "2025-06-18",
      "last-event-id": lastEventId,
    },
    signal: AbortSignal.timeout(5_000),
  });
  const replay = await response.text();
  const sameSessionNewCall = await post(
    other,
    {
      jsonrpc: "2.0",
      id: 11,
      method: "tools/call",
      params: { name: "list_folders", arguments: {} },
    },
    initial.session,
  );
  const locker = spawn(
    "python",
    [
      "-u",
      "-c",
      'import sqlite3,sys; connection=sqlite3.connect(sys.argv[1]); connection.execute("PRAGMA journal_mode=DELETE"); connection.execute("BEGIN EXCLUSIVE"); print("locked",flush=True); sys.stdin.readline(); connection.rollback(); connection.close()',
      path.join(root, "data/index/full-stack.sqlite"),
    ],
    { cwd: root, stdio: ["pipe", "pipe", "pipe"] },
  );
  const lockerExit = new Promise((resolve, reject) => {
    locker.once("error", reject);
    locker.once("exit", resolve);
  });
  let lockDiagnostics = "";
  locker.stderr.on("data", (chunk) => {
    lockDiagnostics += chunk;
  });
  let duplicate;
  try {
    const locked = await Promise.race([
      new Promise((resolve) =>
        locker.stdout.once("data", (chunk) => resolve(String(chunk))),
      ),
      lockerExit.then(() => {
        throw new Error(`Locker exited: ${lockDiagnostics}`);
      }),
      delay(5_000).then(() => {
        throw new Error("Content lock was not acquired");
      }),
    ]);
    assert.equal(locked.trim(), "locked");
    const headers = {
      cookie: owner,
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      "mcp-session-id": initial.session,
      "mcp-protocol-version": "2025-06-18",
    };
    const pending = await fetch(`${base}/mcp`, {
      method: "POST",
      headers,
      signal: AbortSignal.timeout(8_000),
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 20,
        method: "tools/call",
        params: {
          name: "search_articles",
          arguments: { db: "full-stack.sqlite", q: "Evidence" },
        },
      }),
    });
    const pendingReader = pending.body.getReader();
    const prime = await pendingReader.read();
    assert(new TextDecoder().decode(prime.value).includes("retry:"));
    const second = await fetch(`${base}/mcp`, {
      method: "POST",
      headers,
      signal: AbortSignal.timeout(8_000),
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 20,
        method: "ping",
        params: {},
      }),
    });
    locker.stdin.end("\n");
    assert.equal(await lockerExit, 0);
    let pendingText = new TextDecoder().decode(prime.value);
    let pendingError = null;
    const firstComplete = (async () => {
      try {
        for (;;) {
          const chunk = await pendingReader.read();
          if (chunk.done) break;
          pendingText += new TextDecoder().decode(chunk.value);
        }
      } catch (error) {
        pendingError = error.name;
      }
    })();
    let secondText = "";
    let secondError = null;
    try {
      secondText = await second.text();
    } catch (error) {
      secondError = error.name;
    }
    await firstComplete;
    duplicate = {
      pendingStatus: pending.status,
      pendingBody: pendingText,
      pendingError,
      duplicateStatus: second.status,
      duplicateBody: secondText,
      duplicateError: secondError,
    };
  } finally {
    if (locker.exitCode === null) {
      locker.stdin.end("\n");
      await lockerExit;
    }
  }
  const deletion = await fetch(`${base}/mcp`, {
    method: "DELETE",
    headers: {
      cookie: other,
      "mcp-session-id": initial.session,
      "mcp-protocol-version": "2025-06-18",
    },
    signal: AbortSignal.timeout(5_000),
  });
  const report = {
    baseline: BASELINE,
    syntheticOnly: true,
    ownerFolder: "Reading",
    ownerToolResponse: { status: folders.status, body: folders.body },
    replayEventId: lastEventId,
    otherUserIndependentRest: {
      status: independent.status,
      body: independentBody,
    },
    otherUserNewCall: {
      status: sameSessionNewCall.status,
      body: sameSessionNewCall.body,
    },
    otherUserReplay: {
      status: response.status,
      body: replay,
      containsOwnerFolder: replay.includes("Reading"),
    },
    otherUserDelete: deletion.status,
    overlappingDuplicateId: duplicate,
    httpBoundary,
    interpretation:
      "Security observation only; do not adopt cross-user data exposure as a compatibility expectation. Random session/cookie values are deliberately omitted.",
  };
  await deletion.arrayBuffer();
  await fs.writeFile(
    path.join(
      WORKSPACE_ROOT,
      "output/migration/execution/t02-mcp-rust-boundary.json",
    ),
    JSON.stringify(report, null, 2) + "\n",
  );
  process.stdout.write(JSON.stringify(report, null, 2) + "\n");
} finally {
  if (child && child.exitCode === null && child.signalCode === null) {
    if (process.platform === "win32")
      spawnSync("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], {
        timeout: 5_000,
        stdio: "ignore",
      });
    else child.kill("SIGTERM");
    await Promise.race([
      exited,
      delay(5_000).then(() => {
        throw new Error("Fixture service cleanup timed out");
      }),
    ]);
  }
  assert.equal(path.dirname(root), path.resolve(os.tmpdir()));
  assert(!(await fs.lstat(root)).isSymbolicLink());
  assert.equal(
    await fs.readFile(path.join(root, ".litradar-e2e-root"), "utf8"),
    "litradar-full-stack-e2e-v1\n",
  );
  await fs.rm(root, { recursive: true });
}
