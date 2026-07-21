/**
 * Start one exact local image under hardened settings and verify its public runtime contract.
 */

import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { randomBytes } from "node:crypto";
import { fileURLToPath } from "node:url";

const WORKSPACE_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const REPORT_ROOT = path.join(
  WORKSPACE_ROOT,
  "test-results",
  "container-smoke",
);
const SUMMARY_PATH = path.join(REPORT_ROOT, "summary.json");
const FAILURE_LOG_PATH = path.join(REPORT_ROOT, "failure.log");
const TEMP_MARKER_FILE = ".litradar-container-smoke-root";
const TEMP_MARKER_CONTENT = "litradar-container-smoke-v1\n";
const COMMAND_TIMEOUT_MS = 60_000;
const READY_TIMEOUT_MS = 60_000;
const POLL_INTERVAL_MS = 250;

let activeChild;
let containerName;
let hostPort;
let secretRoot;
let shutdownSignal;
let volumeName;

/**
 * Wait for a bounded interval.
 *
 * @param {number} durationMs - Delay in milliseconds.
 * @returns {Promise<void>} Promise resolved after the delay.
 */
function delay(durationMs) {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

/**
 * Redact the temporary host path from retained diagnostics.
 *
 * @param {string} value - Raw diagnostic text.
 * @returns {string} Redacted text.
 */
function sanitizeDiagnostic(value) {
  if (!secretRoot) {
    return value;
  }
  return value.split(secretRoot).join("<secret-root>");
}

/**
 * Wait for one child process to exit.
 *
 * @param {import('node:child_process').ChildProcess} child - Spawned child.
 * @returns {Promise<{code: number | null, signal: NodeJS.Signals | null}>} Exit details.
 */
function waitForExit(child) {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
}

/**
 * Terminate the active command process tree.
 *
 * @returns {void}
 */
function terminateActiveChild() {
  if (
    !activeChild ||
    activeChild.exitCode !== null ||
    activeChild.signalCode !== null
  ) {
    return;
  }
  if (process.platform === "win32" && activeChild.pid) {
    const killer = spawn(
      "taskkill",
      ["/pid", String(activeChild.pid), "/t", "/f"],
      {
        shell: false,
        stdio: "ignore",
      },
    );
    killer.once("error", () => undefined);
    return;
  }
  activeChild.kill(shutdownSignal ?? "SIGTERM");
}

/**
 * Run Docker without echoing mount paths or other arguments.
 *
 * @param {string[]} args - Docker arguments.
 * @param {{allowFailure?: boolean, timeoutMs?: number}} [options={}] - Command options.
 * @returns {Promise<{code: number, stdout: string, stderr: string}>} Captured command result.
 */
async function runDocker(args, options = {}) {
  const timeoutMs = options.timeoutMs ?? COMMAND_TIMEOUT_MS;
  activeChild = spawn("docker", args, {
    cwd: WORKSPACE_ROOT,
    env: process.env,
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  activeChild.stdout.on("data", (chunk) => {
    stdout += String(chunk);
  });
  activeChild.stderr.on("data", (chunk) => {
    stderr += String(chunk);
  });
  let didTimeout = false;
  const timeout = setTimeout(() => {
    didTimeout = true;
    terminateActiveChild();
  }, timeoutMs);
  let result;
  try {
    result = await waitForExit(activeChild);
  } finally {
    clearTimeout(timeout);
    activeChild = undefined;
  }
  const captured = {
    code: result.code ?? 1,
    stdout: sanitizeDiagnostic(stdout.trim()),
    stderr: sanitizeDiagnostic(stderr.trim()),
  };
  if (shutdownSignal) {
    throw new Error(`Docker command interrupted by ${shutdownSignal}`);
  }
  if (didTimeout) {
    throw new Error(`Docker command exceeded ${timeoutMs}ms`);
  }
  if (!options.allowFailure && captured.code !== 0) {
    throw new Error(
      `Docker command failed with exit code ${captured.code}: ${captured.stderr}`,
    );
  }
  return captured;
}

/**
 * Assert one smoke-test invariant.
 *
 * @param {boolean} condition - Required condition.
 * @param {string} message - Failure message.
 * @returns {void}
 */
function assertInvariant(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

/**
 * Fetch one runtime endpoint with a short per-request timeout.
 *
 * @param {string} url - Loopback URL.
 * @returns {Promise<Response>} HTTP response.
 */
function fetchRuntime(url) {
  return fetch(url, { signal: AbortSignal.timeout(2_000) });
}

/**
 * Wait for the container readiness endpoint or fail when the container exits.
 *
 * @param {string} baseUrl - Published loopback base URL.
 * @returns {Promise<void>} Promise resolved after readiness.
 */
async function waitForReadiness(baseUrl) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastError = "readiness endpoint did not respond";
  while (Date.now() < deadline) {
    if (shutdownSignal) {
      throw new Error(`received ${shutdownSignal}`);
    }
    try {
      const response = await fetchRuntime(`${baseUrl}/health/ready`);
      if (response.ok) {
        return;
      }
      lastError = `readiness returned ${response.status}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    const state = await runDocker(
      ["inspect", "--format", "{{.State.Status}}", containerName],
      {
        allowFailure: true,
      },
    );
    if (state.code !== 0 || state.stdout !== "running") {
      throw new Error(
        `container exited before readiness: ${state.stdout || state.stderr}`,
      );
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`container readiness timed out: ${lastError}`);
}

/**
 * Parse the published IPv4 host port from Docker output.
 *
 * @param {string} value - `docker port` output.
 * @returns {number} Published host port.
 */
function parsePublishedPort(value) {
  const match = /127\.0\.0\.1:(\d+)/.exec(value);
  if (!match) {
    throw new Error(`unexpected published port output: ${value}`);
  }
  return Number(match[1]);
}

/**
 * Determine whether the published loopback port has closed.
 *
 * @param {number} port - Host port.
 * @returns {Promise<boolean>} True when no listener accepts a connection.
 */
function isPortClosed(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    socket.setTimeout(500);
    socket.once("connect", () => {
      socket.destroy();
      resolve(false);
    });
    socket.once("error", () => resolve(true));
    socket.once("timeout", () => {
      socket.destroy();
      resolve(true);
    });
  });
}

/**
 * Wait for the published port to close after container removal.
 *
 * @returns {Promise<boolean>} True when closure was observed.
 */
async function waitForPortClosure() {
  if (!hostPort) {
    return true;
  }
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await isPortClosed(hostPort)) {
      return true;
    }
    await delay(POLL_INTERVAL_MS);
  }
  return false;
}

/**
 * Remove the marker-guarded temporary secret root.
 *
 * @returns {Promise<boolean>} True when no temporary root remains.
 */
async function removeSecretRoot() {
  if (!secretRoot) {
    return true;
  }
  const metadata = await fs.lstat(secretRoot);
  assertInvariant(
    metadata.isDirectory() && !metadata.isSymbolicLink(),
    "unsafe secret root type",
  );
  const [realRoot, realTemp] = await Promise.all([
    fs.realpath(secretRoot),
    fs.realpath(os.tmpdir()),
  ]);
  const relativeRoot = path.relative(realTemp, realRoot);
  assertInvariant(
    Boolean(relativeRoot) &&
      !relativeRoot.startsWith("..") &&
      !path.isAbsolute(relativeRoot),
    "unsafe secret root location",
  );
  const markerPath = path.join(realRoot, TEMP_MARKER_FILE);
  const markerMetadata = await fs.lstat(markerPath);
  assertInvariant(
    markerMetadata.isFile() && !markerMetadata.isSymbolicLink(),
    "unsafe secret root marker",
  );
  assertInvariant(
    (await fs.readFile(markerPath, "utf8")) === TEMP_MARKER_CONTENT,
    "invalid secret root marker",
  );
  await fs.rm(realRoot, { recursive: true, force: false });
  secretRoot = undefined;
  return true;
}

/**
 * Remove the managed container, named volume, listener, and secret root.
 *
 * @returns {Promise<{containerRemoved: boolean, volumeRemoved: boolean, portClosed: boolean, secretRootRemoved: boolean, errors: string[]}>} Cleanup report.
 */
async function cleanup() {
  const errors = [];
  let containerRemoved = !containerName;
  let volumeRemoved = !volumeName;
  if (containerName) {
    const result = await runDocker(["rm", "--force", containerName], {
      allowFailure: true,
    }).catch((error) => ({ code: 1, stderr: error.message }));
    containerRemoved =
      result.code === 0 || /No such container/i.test(result.stderr);
    if (!containerRemoved) {
      errors.push(`container cleanup: ${result.stderr}`);
    }
  }
  const portClosed = await waitForPortClosure();
  if (!portClosed) {
    errors.push(`published port ${hostPort} remained open`);
  }
  if (volumeName) {
    const result = await runDocker(["volume", "rm", "--force", volumeName], {
      allowFailure: true,
    }).catch((error) => ({ code: 1, stderr: error.message }));
    volumeRemoved = result.code === 0 || /No such volume/i.test(result.stderr);
    if (!volumeRemoved) {
      errors.push(`volume cleanup: ${result.stderr}`);
    }
  }
  let secretRootRemoved = false;
  try {
    secretRootRemoved = await removeSecretRoot();
  } catch (error) {
    errors.push(
      `secret cleanup: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
  return {
    containerRemoved,
    volumeRemoved,
    portClosed,
    secretRootRemoved,
    errors,
  };
}

/**
 * Execute the exact-image security and HTTP probes.
 *
 * @param {string} imageReference - Local image reference.
 * @returns {Promise<Record<string, unknown>>} Successful smoke report before cleanup.
 */
async function runSmoke(imageReference) {
  const suffix = `${process.pid}-${randomBytes(4).toString("hex")}`;
  containerName = `litradar-smoke-${suffix}`;
  volumeName = `litradar-smoke-data-${suffix}`;
  secretRoot = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-container-smoke-"),
  );
  await Promise.all([
    fs.writeFile(path.join(secretRoot, TEMP_MARKER_FILE), TEMP_MARKER_CONTENT, {
      flag: "wx",
    }),
    fs.writeFile(path.join(secretRoot, "litradar_key"), randomBytes(32), {
      flag: "wx",
      mode: 0o600,
    }),
  ]);

  const imageId = (
    await runDocker(["image", "inspect", "--format", "{{.Id}}", imageReference])
  ).stdout;
  assertInvariant(
    imageId.startsWith("sha256:"),
    "local image did not resolve to a content ID",
  );
  await runDocker(["volume", "create", volumeName]);
  await runDocker([
    "run",
    "--detach",
    "--name",
    containerName,
    "--read-only",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges",
    "--tmpfs",
    "/tmp:rw,noexec,nosuid,nodev,size=64m",
    "--mount",
    `type=volume,source=${volumeName},target=/app/data`,
    "--mount",
    `type=bind,source=${secretRoot},target=/run/secrets,readonly`,
    "--publish",
    "127.0.0.1::8000",
    imageReference,
  ]);

  const portOutput = await runDocker(["port", containerName, "8000/tcp"]);
  hostPort = parsePublishedPort(portOutput.stdout);
  const baseUrl = `http://127.0.0.1:${hostPort}`;
  await waitForReadiness(baseUrl);

  const [rootResponse, openApiResponse, inspectResult] = await Promise.all([
    fetchRuntime(`${baseUrl}/`),
    fetchRuntime(`${baseUrl}/openapi.json`),
    runDocker(["inspect", containerName]),
  ]);
  assertInvariant(
    rootResponse.ok,
    `root endpoint returned ${rootResponse.status}`,
  );
  assertInvariant(
    openApiResponse.ok,
    `OpenAPI endpoint returned ${openApiResponse.status}`,
  );
  const rootBody = await rootResponse.text();
  const openApi = await openApiResponse.json();
  assertInvariant(
    rootBody.includes("LitRadar"),
    "root endpoint omitted the application marker",
  );
  assertInvariant(
    openApi.openapi === "3.1.0",
    "OpenAPI endpoint returned an unexpected document",
  );
  assertInvariant(
    Boolean(openApi.paths?.["/health/ready"]),
    "OpenAPI omitted readiness",
  );

  const [inspection] = JSON.parse(inspectResult.stdout);
  const dataMount = inspection.Mounts.find(
    (mount) => mount.Destination === "/app/data",
  );
  const secretMount = inspection.Mounts.find(
    (mount) => mount.Destination === "/run/secrets",
  );
  assertInvariant(
    inspection.Image === imageId,
    "container did not use the inspected image ID",
  );
  assertInvariant(
    inspection.HostConfig.ReadonlyRootfs === true,
    "root filesystem is writable",
  );
  assertInvariant(
    inspection.HostConfig.CapDrop?.some(
      (capability) => capability.toUpperCase() === "ALL",
    ),
    "Linux capabilities were not dropped",
  );
  assertInvariant(
    inspection.HostConfig.SecurityOpt?.some((option) =>
      option.startsWith("no-new-privileges"),
    ),
    "no-new-privileges is missing",
  );
  assertInvariant(
    Boolean(inspection.Config.User) &&
      !["0", "root"].includes(inspection.Config.User),
    "container runs as root",
  );
  assertInvariant(dataMount?.RW === true, "data mount is not writable");
  assertInvariant(secretMount?.RW === false, "secret mount is not read-only");

  return {
    status: "passed",
    imageReference,
    imageId,
    containerUser: inspection.Config.User,
    endpoints: ["/", "/health/ready", "/openapi.json"],
    security: {
      readOnlyRoot: true,
      capabilitiesDropped: true,
      noNewPrivileges: true,
      temporaryFilesystem: true,
      writableDataVolume: true,
      readOnlySecretMount: true,
    },
  };
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    shutdownSignal = signal;
    terminateActiveChild();
  });
}

await fs.mkdir(REPORT_ROOT, { recursive: true });
await fs.rm(FAILURE_LOG_PATH, { force: true });

const args = process.argv.slice(2);
let report;
let failure;

if (args.length !== 1 || !args[0].trim()) {
  failure = new Error(
    "Usage: node scripts/container-smoke.mjs <local-image-reference>",
  );
} else {
  try {
    report = await runSmoke(args[0].trim());
  } catch (error) {
    failure = error instanceof Error ? error : new Error(String(error));
    if (containerName) {
      const logs = await runDocker(["logs", "--tail", "200", containerName], {
        allowFailure: true,
      }).catch(() => ({ stdout: "", stderr: "" }));
      const safeLogs = sanitizeDiagnostic(
        [logs.stdout, logs.stderr].filter(Boolean).join("\n"),
      );
      if (safeLogs) {
        failure = new Error(`${failure.message}\n${safeLogs}`);
      }
    }
  }
}

const cleanupReport = await cleanup();
if (cleanupReport.errors.length > 0 && !failure) {
  failure = new Error(cleanupReport.errors.join("\n"));
}
const finalReport = {
  ...(report ?? { status: "failed", imageReference: args[0] ?? null }),
  status: failure ? "failed" : "passed",
  cleanup: cleanupReport,
};
await fs.writeFile(
  SUMMARY_PATH,
  `${JSON.stringify(finalReport, null, 2)}\n`,
  "utf8",
);

if (failure) {
  const safeFailure = sanitizeDiagnostic(failure.stack ?? failure.message);
  await fs.writeFile(FAILURE_LOG_PATH, `${safeFailure}\n`, "utf8");
  process.stderr.write(`[container-smoke] ${safeFailure}\n`);
  process.exitCode =
    shutdownSignal === "SIGINT" ? 130 : shutdownSignal === "SIGTERM" ? 143 : 1;
} else {
  process.stdout.write(`${JSON.stringify(finalReport)}\n`);
}
