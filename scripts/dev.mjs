/**
 * Run the local Rust API and Next.js frontend with one shared lifecycle.
 *
 * @module dev
 */

import { spawn } from "node:child_process";
import { access, stat } from "node:fs/promises";
import { createServer } from "node:net";
import path from "node:path";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const WORKSPACE_ROOT = path.resolve(
  fileURLToPath(new URL("..", import.meta.url)),
);
const APP_ROOT = path.join(WORKSPACE_ROOT, "app");
const NEXT_CLI = path.join(
  APP_ROOT,
  "node_modules",
  "next",
  "dist",
  "bin",
  "next",
);
const READY_TIMEOUT_MS = 300_000;
const SHUTDOWN_TIMEOUT_MS = 10_000;
const CHILDREN = [];

let isStopping = false;
let receivedSignal;
let resolveInterrupt;
const INTERRUPTED = new Promise((resolve) => {
  resolveInterrupt = resolve;
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    receivedSignal = signal;
    isStopping = true;
    resolveInterrupt({ signal });
  });
}

/**
 * Reject an occupied development port without stopping its current owner.
 *
 * @param {number} port - Required loopback port.
 * @returns {Promise<void>} Resolves once the temporary listener is closed.
 */
async function checkPort(port) {
  await new Promise((resolve, reject) => {
    const listener = createServer();
    listener.once("error", () =>
      reject(new Error(`Port ${port} is already in use.`)),
    );
    listener.listen({ host: "127.0.0.1", port, exclusive: true }, () => {
      listener.close((error) => (error ? reject(error) : resolve()));
    });
  });
}

/**
 * Start one owned process tree and immediately observe startup errors and exits.
 *
 * @param {string} label - Service name used in diagnostics.
 * @param {string} command - Executable path or name.
 * @param {string[]} args - Literal command arguments.
 * @param {string} cwd - Child working directory.
 * @param {boolean} shouldCaptureOutput - Whether Cargo JSON output should be parsed.
 * @returns {{child: import('node:child_process').ChildProcess, exited: Promise<object>}} Child lifecycle.
 */
function startChild(label, command, args, cwd, shouldCaptureOutput = false) {
  const child = spawn(command, args, {
    cwd,
    stdio: shouldCaptureOutput ? ["ignore", "pipe", "inherit"] : "inherit",
    shell: false,
    detached: process.platform !== "win32",
  });
  const exited = new Promise((resolve) => {
    child.once("error", (error) => resolve({ label, error }));
    child.once("close", (code, signal) => resolve({ label, code, signal }));
  });
  const entry = { child, exited };
  CHILDREN.push(entry);
  return entry;
}

/**
 * Build the backend and discover its executable through Cargo's artifact output.
 *
 * @returns {Promise<string | undefined>} Built executable, or undefined after interruption.
 */
async function buildBackend() {
  const build = startChild(
    "Rust build",
    "cargo",
    [
      "build",
      "--locked",
      "--bin",
      "litradar",
      "--message-format=json-render-diagnostics",
    ],
    WORKSPACE_ROOT,
    true,
  );
  let executable;
  let outputError;
  const output = createInterface({ input: build.child.stdout });
  output.on("line", (line) => {
    try {
      const artifact = JSON.parse(line);
      if (
        artifact.reason === "compiler-artifact" &&
        artifact.target.name === "litradar" &&
        artifact.executable
      ) {
        executable = artifact.executable;
      }
    } catch (error) {
      outputError = error;
    }
  });
  const result = await Promise.race([build.exited, INTERRUPTED]);
  if (isStopping) return;
  if (result.error || result.code !== 0 || outputError || !executable) {
    throw new Error(
      `Rust build failed: ${result.error?.message ?? outputError?.message ?? result.code}`,
    );
  }
  CHILDREN.splice(CHILDREN.indexOf(build), 1);
  return executable;
}

/**
 * Wait at most five minutes for a service started by this command.
 *
 * @param {string} url - Local readiness endpoint.
 * @returns {Promise<void>} Resolves on readiness or interruption.
 */
async function waitForReady(url) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (!isStopping && Date.now() < deadline) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(2_000) });
      await response.body?.cancel();
      if (response.ok) return;
    } catch {
      if (isStopping) return;
    }
    await delay(250);
  }
  if (!isStopping) throw new Error(`Service readiness timed out: ${url}`);
}

/**
 * Stop an owned process tree, allowing console interrupts to finish gracefully.
 *
 * @param {ReturnType<typeof startChild>} entry - Owned child and its exit promise.
 * @returns {Promise<void>} Resolves after shutdown or throws if termination fails.
 */
async function stopChild({ child, exited }) {
  if (!child.pid) return;
  const hasExited = () => child.exitCode !== null || child.signalCode !== null;
  if (process.platform === "win32") {
    if (receivedSignal === "SIGINT") {
      await Promise.race([
        exited,
        delay(SHUTDOWN_TIMEOUT_MS, undefined, { ref: false }),
      ]);
    }
    if (hasExited()) return;
    const killer = spawn("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
      stdio: "ignore",
      shell: false,
      windowsHide: true,
    });
    await new Promise((resolve, reject) => {
      killer.once("error", reject);
      killer.once("exit", (code) => {
        if (code === 0 || hasExited()) resolve();
        else reject(new Error(`Could not stop process tree ${child.pid}.`));
      });
    });
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
    await Promise.race([
      exited,
      delay(SHUTDOWN_TIMEOUT_MS, undefined, { ref: false }),
    ]);
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }
  await Promise.race([
    exited,
    delay(SHUTDOWN_TIMEOUT_MS, undefined, { ref: false }).then(() => {
      throw new Error(`Process ${child.pid} did not exit after shutdown.`);
    }),
  ]);
}

/**
 * Prepare the requested data root and supervise both local development services.
 *
 * @returns {Promise<void>} Resolves when the user stops the command.
 */
async function main() {
  const args = process.argv.slice(2);
  const usage = "Usage: node scripts/dev.mjs [--project-root PATH]";
  if (args.length === 1 && ["--help", "-h"].includes(args[0])) {
    process.stdout.write(`${usage}\nPress Ctrl+C to stop both services.\n`);
    return;
  }
  if (
    args.length !== 0 &&
    (args.length !== 2 || args[0] !== "--project-root")
  ) {
    throw new Error(usage);
  }
  const projectRoot =
    args.length === 2 ? path.resolve(args[1]) : WORKSPACE_ROOT;
  const secretKeyFile = path.join(projectRoot, "secrets", "litradar.key");
  await access(NEXT_CLI).catch(() => {
    throw new Error(
      "Install frontend dependencies with pnpm --dir app install --frozen-lockfile.",
    );
  });
  const keyMetadata = await stat(secretKeyFile);
  if (!keyMetadata.isFile() || keyMetadata.size !== 32) {
    throw new Error(`Expected a 32-byte deployment key: ${secretKeyFile}`);
  }
  await Promise.all([checkPort(8000), checkPort(8001)]);
  if (isStopping) return;
  const executable = await buildBackend();
  if (isStopping) return;
  const backend = startChild(
    "Rust API",
    executable,
    [
      "serve",
      "--development",
      "--host",
      "127.0.0.1",
      "--port",
      "8001",
      "--project-root",
      projectRoot,
      "--secret-key-file",
      secretKeyFile,
    ],
    WORKSPACE_ROOT,
  );
  const frontend = startChild(
    "Next.js",
    process.execPath,
    [NEXT_CLI, "dev", "--hostname", "127.0.0.1", "--port", "8000"],
    APP_ROOT,
  );
  const stopped = Promise.race([INTERRUPTED, backend.exited, frontend.exited]);
  const ready = Promise.all([
    waitForReady("http://127.0.0.1:8001/health/ready"),
    waitForReady("http://127.0.0.1:8000/login"),
  ]).then(() => undefined);
  let outcome = await Promise.race([ready, stopped]);
  if (!outcome && !isStopping) {
    process.stdout.write(
      "[dev] Ready: http://localhost:8000 (API: 127.0.0.1:8001). Press Ctrl+C to stop both.\n",
    );
    outcome = await stopped;
  }
  if (outcome?.label && !receivedSignal) {
    throw new Error(
      `${outcome.label} exited unexpectedly: ${outcome.error?.message ?? outcome.signal ?? outcome.code}`,
    );
  }
}

try {
  await main();
} catch (error) {
  process.stderr.write(`[dev] ${error.message}\n`);
  process.exitCode = 1;
} finally {
  isStopping = true;
  const results = await Promise.allSettled(CHILDREN.map(stopChild));
  for (const result of results) {
    if (result.status === "rejected") {
      process.stderr.write(`[dev] ${result.reason.message}\n`);
      process.exitCode = 1;
    }
  }
}
