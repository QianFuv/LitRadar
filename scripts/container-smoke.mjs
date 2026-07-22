/**
 * Start one exact local image under hardened settings and verify its public runtime contract.
 */

import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
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
const COMMAND_TIMEOUT_MS = 60_000;
const READY_TIMEOUT_MS = 60_000;
const POLL_INTERVAL_MS = 250;
const REMOVED_APPLICATION_ENVIRONMENT_NAMES = [
  "NEXT_PUBLIC_API_URL",
  "INTERNAL_API_URL",
  "LITRADAR_BUNDLED_META_DIR",
  "LITRADAR_LOG_FORMAT",
  "LITRADAR_LOG_FILTER",
  "LITRADAR_PARENT_RUN_ID",
];

let activeChild;
let containerName;
let hostPort;
let secretInitializerName;
let secretVolumeName;
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
    stdout: stdout.trim(),
    stderr: stderr.trim(),
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
 * Resolve the published port while preserving an early container-exit diagnosis.
 *
 * @returns {Promise<number>} Published loopback port.
 */
async function resolvePublishedPort() {
  const portResult = await runDocker(["port", containerName, "8000/tcp"], {
    allowFailure: true,
  });
  if (portResult.code === 0) {
    return parsePublishedPort(portResult.stdout);
  }
  const state = await runDocker(
    ["inspect", "--format", "{{.State.Status}}", containerName],
    { allowFailure: true },
  );
  if (state.code !== 0 || state.stdout !== "running") {
    throw new Error(
      `container exited before port publication: ${state.stdout || state.stderr}`,
    );
  }
  throw new Error(
    `published port unavailable: ${portResult.stderr || portResult.stdout}`,
  );
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
 * Remove one managed container if it still exists.
 *
 * @param {string | undefined} name - Managed container name.
 * @returns {Promise<{removed: boolean, error: string}>} Removal result.
 */
async function removeManagedContainer(name) {
  if (!name) {
    return { removed: true, error: "" };
  }
  const result = await runDocker(["rm", "--force", name], {
    allowFailure: true,
  }).catch((error) => ({ code: 1, stderr: error.message }));
  const removed = result.code === 0 || /No such container/i.test(result.stderr);
  return { removed, error: removed ? "" : result.stderr };
}

/**
 * Remove one managed volume if it still exists.
 *
 * @param {string | undefined} name - Managed volume name.
 * @returns {Promise<{removed: boolean, error: string}>} Removal result.
 */
async function removeManagedVolume(name) {
  if (!name) {
    return { removed: true, error: "" };
  }
  const result = await runDocker(["volume", "rm", "--force", name], {
    allowFailure: true,
  }).catch((error) => ({ code: 1, stderr: error.message }));
  const removed = result.code === 0 || /No such volume/i.test(result.stderr);
  return { removed, error: removed ? "" : result.stderr };
}

/**
 * Remove the managed containers, named volumes, and listener.
 *
 * @returns {Promise<{containerRemoved: boolean, secretInitializerRemoved: boolean, volumeRemoved: boolean, secretVolumeRemoved: boolean, portClosed: boolean, errors: string[]}>} Cleanup report.
 */
async function cleanup() {
  const errors = [];
  const containerRemoval = await removeManagedContainer(containerName);
  if (!containerRemoval.removed) {
    errors.push(`container cleanup: ${containerRemoval.error}`);
  }
  const initializerRemoval = await removeManagedContainer(
    secretInitializerName,
  );
  if (!initializerRemoval.removed) {
    errors.push(`secret initializer cleanup: ${initializerRemoval.error}`);
  }
  const portClosed = await waitForPortClosure();
  if (!portClosed) {
    errors.push(`published port ${hostPort} remained open`);
  }
  const volumeRemoval = await removeManagedVolume(volumeName);
  if (!volumeRemoval.removed) {
    errors.push(`volume cleanup: ${volumeRemoval.error}`);
  }
  const secretVolumeRemoval = await removeManagedVolume(secretVolumeName);
  if (!secretVolumeRemoval.removed) {
    errors.push(`secret volume cleanup: ${secretVolumeRemoval.error}`);
  }
  return {
    containerRemoved: containerRemoval.removed,
    secretInitializerRemoved: initializerRemoval.removed,
    volumeRemoved: volumeRemoval.removed,
    secretVolumeRemoved: secretVolumeRemoval.removed,
    portClosed,
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
  secretInitializerName = `${containerName}-secret-init`;
  volumeName = `litradar-smoke-data-${suffix}`;
  secretVolumeName = `litradar-smoke-secret-${suffix}`;

  const imageId = (
    await runDocker(["image", "inspect", "--format", "{{.Id}}", imageReference])
  ).stdout;
  assertInvariant(
    imageId.startsWith("sha256:"),
    "local image did not resolve to a content ID",
  );
  await runDocker(["volume", "create", volumeName]);
  await runDocker(["volume", "create", secretVolumeName]);
  await runDocker([
    "run",
    "--rm",
    "--name",
    secretInitializerName,
    "--network",
    "none",
    "--read-only",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges",
    "--mount",
    `type=volume,source=${secretVolumeName},target=/app/data`,
    "--entrypoint",
    "/bin/sh",
    imageReference,
    "-c",
    'umask 077; head -c 32 /dev/urandom > /app/data/litradar_key; test "$(wc -c < /app/data/litradar_key)" -eq 32',
  ]);
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
    `type=volume,source=${secretVolumeName},target=/run/secrets,readonly`,
    "--publish",
    "127.0.0.1::8000",
    imageReference,
  ]);

  hostPort = await resolvePublishedPort();
  const baseUrl = `http://127.0.0.1:${hostPort}`;
  await waitForReadiness(baseUrl);

  await runDocker([
    "exec",
    containerName,
    "sh",
    "-c",
    "test -f /app/data/meta/ccf_computer_journals.csv && test -f /app/data/meta/chinese_journals.csv && test -f /app/data/meta/english_journals.csv",
  ]);
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
  const configuredEnvironment = inspection.Config.Env ?? [];
  const removedEnvironmentOverrides = configuredEnvironment.filter((entry) =>
    REMOVED_APPLICATION_ENVIRONMENT_NAMES.some((name) =>
      entry.startsWith(`${name}=`),
    ),
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
  assertInvariant(
    secretMount?.Type === "volume" && secretMount?.Name === secretVolumeName,
    "secret mount is not the managed volume",
  );
  assertInvariant(secretMount?.RW === false, "secret mount is not read-only");
  assertInvariant(
    removedEnvironmentOverrides.length === 0,
    "container declares removed application environment overrides",
  );

  return {
    status: "passed",
    imageReference,
    imageId,
    containerUser: inspection.Config.User,
    endpoints: ["/", "/health/ready", "/openapi.json"],
    managedMetaPrepared: true,
    removedEnvironmentOverrides: [],
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
      const safeLogs = [logs.stdout, logs.stderr].filter(Boolean).join("\n");
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
  const safeFailure = failure.stack ?? failure.message;
  await fs.writeFile(FAILURE_LOG_PATH, `${safeFailure}\n`, "utf8");
  process.stderr.write(`[container-smoke] ${safeFailure}\n`);
  process.exitCode =
    shutdownSignal === "SIGINT" ? 130 : shutdownSignal === "SIGTERM" ? 143 : 1;
} else {
  process.stdout.write(`${JSON.stringify(finalReport)}\n`);
}
