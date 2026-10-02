/** Capture public CLI and MCP contracts from the verified frozen Rust executable. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as delay } from "node:timers/promises";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "./oracle.mjs";

const COMMANDS = [
  [],
  ["serve"],
  ["admin"],
  ["admin", "bootstrap"],
  ["admin", "secrets"],
  ["admin", "secrets", "migrate"],
  ["admin", "secrets", "verify"],
  ["admin", "secrets", "rotate"],
  ["admin", "backup"],
  ["admin", "backup", "create"],
  ["admin", "backup", "verify"],
  ["admin", "backup", "restore"],
  ["admin", "index"],
  ["admin", "index", "optimize-storage"],
  ["index"],
  ["cfp"],
  ["cfp", "import"],
  ["cfp", "refresh"],
  ["notify"],
  ["push"],
  ["scheduler"],
  ["scheduler", "validate"],
  ["scheduler", "run-once"],
  ["scheduler", "dry-run-once"],
  ["openapi"],
];

/**
 * Run a frozen executable with bounded output and only disposable paths.
 * @param {string} executable - Verified executable path.
 * @param {string[]} args - Literal command arguments.
 * @param {string} cwd - Owned disposable working directory.
 * @returns {object} Exact output and exit status.
 */
function execute(executable, args, cwd) {
  const result = spawnSync(executable, args, {
    cwd,
    encoding: "utf8",
    timeout: 30_000,
    maxBuffer: 8 * 1024 * 1024,
    shell: false,
    env: { ...process.env, RUST_LOG: "error", NO_COLOR: "1" },
  });
  if (result.error) throw result.error;
  assert.equal(result.signal, null);
  return {
    args,
    exitCode: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

/**
 * Reserve a loopback port; subsequent readiness detects a failed bind.
 * @returns {Promise<number>} Port.
 */
async function portNumber() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

/**
 * Decode one completed MCP JSON response, including its SSE envelope.
 * @param {string} body - Exact wire payload.
 * @returns {object} JSON-RPC message.
 */
function rpcMessage(body) {
  if (body.startsWith("{")) return JSON.parse(body);
  const messages = body
    .split(/\r?\n\r?\n/)
    .map((event) =>
      event
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n"),
    )
    .filter((data) => data.trim())
    .map((data) => JSON.parse(data));
  assert.equal(messages.length, 1, "Expected exactly one completed MCP result");
  return messages[0];
}

assert.deepEqual(process.argv.slice(2), ["--baseline", BASELINE]);
const oracle = await loadOracle(BASELINE);
const output = path.join(WORKSPACE_ROOT, "tests/data/migration/surfaces.json");
await assert.rejects(
  fs.lstat(output),
  { code: "ENOENT" },
  "Explicit capture is immutable",
);
const root = await fs.mkdtemp(
  path.join(os.tmpdir(), "litradar-migration-surfaces-"),
);
let child;
let exited;
let stderr = "";
try {
  await fs.writeFile(
    path.join(root, ".litradar-e2e-root"),
    "litradar-full-stack-e2e-v1\n",
    { flag: "wx" },
  );
  await fs.writeFile(path.join(root, "secret.key"), Buffer.alloc(32, 42), {
    mode: 0o600,
    flag: "wx",
  });
  const application = path.join(
    oracle.directory,
    oracle.manifest.binaries.application,
  );
  const cli = COMMANDS.map((command) =>
    execute(application, [...command, "--help"], root),
  );
  cli.forEach((entry) => assert.equal(entry.exitCode, 0, entry.args.join(" ")));
  const invalidArgument = execute(
    application,
    ["--migration-invalid-option"],
    root,
  );
  assert.equal(invalidArgument.exitCode, 1);
  const seeded = execute(
    path.join(oracle.directory, oracle.manifest.binaries.fullStackFixture),
    ["--project-root", root],
    root,
  );
  assert.equal(seeded.exitCode, 0, seeded.stderr);
  assert.equal(JSON.parse(seeded.stdout).article_count, 2);
  const port = await portNumber();
  const base = `http://127.0.0.1:${port}`;
  const started = performance.now();
  child = spawn(
    application,
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
    {
      cwd: root,
      shell: false,
      env: { ...process.env, RUST_LOG: "error" },
      stdio: ["ignore", "ignore", "pipe"],
    },
  );
  child.stderr.on("data", (chunk) => {
    stderr += String(chunk);
    assert(stderr.length <= 1024 * 1024, "Unbounded service diagnostic output");
  });
  exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", resolve);
  });
  let ready = false;
  while (performance.now() - started < 30_000) {
    assert.equal(child.exitCode, null, stderr);
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
  assert(ready, `Frozen service did not become ready: ${stderr}`);
  const startupMs = performance.now() - started;
  const login = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username: "fullstack_admin",
      password: "FullStackAdmin!2026",
    }),
    signal: AbortSignal.timeout(10_000),
  });
  assert.equal(login.status, 200, await login.text());
  const cookie = login.headers
    .getSetCookie()
    .find((value) => value.startsWith("litradar_session="))
    .split(";")[0];
  /**
   * Send one bounded authenticated MCP POST and retain exact public wire bytes.
   * @param {object} payload - JSON-RPC request.
   * @param {string} [session] - Live session token, excluded from persisted capture.
   * @returns {Promise<object>} Wire response plus live response headers.
   */
  async function rpc(payload, session) {
    const headers = {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      cookie,
    };
    if (session)
      Object.assign(headers, {
        "mcp-session-id": session,
        "mcp-protocol-version": "2025-06-18",
      });
    const response = await fetch(`${base}/mcp`, {
      method: "POST",
      headers,
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(10_000),
    });
    return {
      status: response.status,
      body: await response.text(),
      headers: response.headers,
    };
  }
  const initialized = await rpc({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "litradar-migration", version: "1" },
    },
  });
  assert.equal(initialized.status, 200, initialized.body);
  const session = initialized.headers.get("mcp-session-id");
  assert(session);
  const notification = await rpc(
    { jsonrpc: "2.0", method: "notifications/initialized" },
    session,
  );
  assert.equal(notification.status, 202, notification.body);
  const listed = await rpc(
    { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
    session,
  );
  assert.equal(listed.status, 200, listed.body);
  const tools = rpcMessage(listed.body).result.tools;
  assert.equal(tools.length, 13);
  const workloadStart = performance.now();
  for (let request = 0; request < 100; request += 1) {
    const response = await fetch(`${base}/api/meta/databases`, {
      headers: { cookie },
      signal: AbortSignal.timeout(2_000),
    });
    assert.equal(response.status, 200);
    await response.arrayBuffer();
  }
  const requestMs = performance.now() - workloadStart;
  let memory;
  if (process.platform === "win32") {
    const sampled = execute(
      "powershell.exe",
      ["-NoProfile", "-Command", `(Get-Process -Id ${child.pid}).WorkingSet64`],
      root,
    );
    assert.equal(sampled.exitCode, 0);
    memory = {
      workingSetBytes: Number(sampled.stdout.trim()),
      sampling:
        "Get-Process WorkingSet64 after 100 sequential database-list requests",
    };
    assert(
      Number.isSafeInteger(memory.workingSetBytes) &&
        memory.workingSetBytes > 0,
    );
  }
  await fs.writeFile(
    output,
    `${JSON.stringify(
      {
        format: 1,
        baseline: BASELINE,
        platform: process.platform,
        oracleManifestSha256: digest(
          await fs.readFile(path.join(oracle.directory, "manifest.json")),
        ),
        cli,
        invalidArgument,
        mcp: {
          initialize: rpcMessage(initialized.body),
          tools,
          initializeWire: initialized.body,
          initializedStatus: notification.status,
          initializedBody: notification.body,
          toolsListWire: listed.body,
        },
        resourceBaseline: {
          profile:
            "debug; synthetic full-stack fixture: 2 articles; loopback; development mode",
          startupMs,
          sequentialDatabaseRequests: 100,
          requestMs,
          ...memory,
          cancellation:
            "Measured separately by phase-owned process tests; forced harness teardown is not graceful cancellation evidence",
        },
        exclusions: [
          "Random authenticated session/cookie values are runtime credentials and are not serialized; their policies remain separate parity obligations",
          "CLI help is wire evidence, not a substitute for command side-effect tests",
          "Debug Windows measurements are not Linux production or release-build performance claims",
        ],
      },
      null,
      2,
    )}\n`,
    { flag: "wx" },
  );
  process.stdout.write(
    "Captured 25 CLI help surfaces, parser error, 13 complete MCP schemas, and a fixed loopback resource workload.\n",
  );
} finally {
  if (child && child.exitCode === null && child.signalCode === null) {
    if (process.platform === "win32")
      execute("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], root);
    else child.kill("SIGTERM");
    await Promise.race([
      exited,
      delay(5_000).then(() => {
        throw new Error("Service cleanup timed out");
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
