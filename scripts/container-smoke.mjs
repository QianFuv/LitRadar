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
const FAILURE_LOG_MESSAGE =
  "Container smoke failed; inspect the redacted workflow stderr for details.\n";
const COMMAND_TIMEOUT_MS = 60_000;
const IMAGE_PULL_TIMEOUT_MS = 300_000;
const READY_TIMEOUT_MS = 60_000;
const POLL_INTERVAL_MS = 250;
const DIGEST_REFERENCE_PATTERN =
  /^[a-z0-9]+(?:[._-][a-z0-9]+)*(?::[0-9]+)?\/[a-z0-9]+(?:[._/-][a-z0-9]+)*@sha256:[0-9a-f]{64}$/;
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
 * @param {{allowFailure?: boolean, input?: string, timeoutMs?: number}} [options={}] - Command options.
 * @returns {Promise<{code: number, stdout: string, stderr: string}>} Captured command result.
 */
async function runDocker(args, options = {}) {
  const timeoutMs = options.timeoutMs ?? COMMAND_TIMEOUT_MS;
  const hasInput = typeof options.input === "string";
  activeChild = spawn("docker", args, {
    cwd: WORKSPACE_ROOT,
    env: process.env,
    shell: false,
    stdio: [hasInput ? "pipe" : "ignore", "pipe", "pipe"],
  });
  if (hasInput) {
    activeChild.stdin.end(options.input);
  }
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
 * Assert the baseline response security policy without weakening inline scripts.
 *
 * @param {Response} response - Runtime response to inspect.
 * @param {boolean} isHstsExpected - Whether hardened HTTPS mode should emit HSTS.
 * @returns {void}
 */
function assertSecurityHeaders(response, isHstsExpected) {
  const contentSecurityPolicy = response.headers.get("content-security-policy");
  assertInvariant(
    contentSecurityPolicy?.includes("default-src 'self'"),
    "response omitted the default CSP policy",
  );
  assertInvariant(
    contentSecurityPolicy.includes("frame-ancestors 'none'"),
    "response omitted CSP frame denial",
  );
  const scriptDirective = contentSecurityPolicy
    .split(";")
    .map((directive) => directive.trim())
    .find((directive) => directive.startsWith("script-src"));
  assertInvariant(
    scriptDirective?.includes("'self'") &&
      !scriptDirective.includes("'unsafe-inline'"),
    "response CSP weakened inline script protection",
  );
  assertInvariant(
    response.headers.get("x-content-type-options") === "nosniff",
    "response omitted nosniff",
  );
  assertInvariant(
    response.headers.get("referrer-policy") === "same-origin",
    "response returned an unexpected referrer policy",
  );
  assertInvariant(
    response.headers.get("permissions-policy") ===
      "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
    "response returned an unexpected permissions policy",
  );
  assertInvariant(
    response.headers.get("x-frame-options") === "DENY",
    "response omitted frame denial",
  );
  assertInvariant(
    response.headers.has("strict-transport-security") === isHstsExpected,
    "response returned an unexpected HSTS mode",
  );
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
 * Wait for the image-defined Docker health check to become healthy.
 *
 * @returns {Promise<void>} Promise resolved after Docker reports healthy.
 */
async function waitForContainerHealth() {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastState = "missing";
  while (Date.now() < deadline) {
    const state = await runDocker(
      [
        "inspect",
        "--format",
        "{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}",
        containerName,
      ],
      { allowFailure: true },
    );
    lastState = state.stdout || state.stderr;
    if (state.code === 0 && state.stdout === "healthy") {
      return;
    }
    if (state.code !== 0 || ["missing", "unhealthy"].includes(state.stdout)) {
      throw new Error(`container health check failed: ${lastState}`);
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`container health check timed out: ${lastState}`);
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
 * Build hardened service arguments for the managed smoke container.
 *
 * @param {string} imageReference - Exact image reference under test.
 * @param {boolean} areSecureCookiesRequired - Whether startup must enforce secure cookies.
 * @returns {string[]} Docker run arguments.
 */
function buildServiceRunArguments(imageReference, areSecureCookiesRequired) {
  const runArguments = [
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
    "serve",
    "--host",
    "0.0.0.0",
    "--port",
    "8000",
    "--project-root",
    "/app",
    "--secret-key-file",
    "/run/secrets/litradar_key",
  ];
  if (areSecureCookiesRequired) {
    runArguments.push("--require-secure-cookies");
  }
  return runArguments;
}

/**
 * Bootstrap an isolated administrator and persist the secure-cookie setting.
 *
 * @param {string} imageReference - Exact image reference under test.
 * @returns {Promise<void>} Promise resolved after the setting is committed and the setup service is removed.
 */
async function enableSecureCookies(imageReference) {
  const username = "container_smoke_admin";
  const password = `${randomBytes(24).toString("base64url")}Aa1!`;
  await runDocker(
    [
      "run",
      "--rm",
      "--interactive",
      "--network",
      "none",
      "--read-only",
      "--cap-drop",
      "ALL",
      "--security-opt",
      "no-new-privileges",
      "--tmpfs",
      "/tmp:rw,noexec,nosuid,nodev,size=64m",
      "--mount",
      `type=volume,source=${volumeName},target=/app/data`,
      imageReference,
      "admin",
      "bootstrap",
      "--username",
      username,
      "--password-stdin",
      "--project-root",
      "/app",
    ],
    { input: `${password}\n` },
  );
  await runDocker(buildServiceRunArguments(imageReference, false));
  hostPort = await resolvePublishedPort();
  const baseUrl = `http://127.0.0.1:${hostPort}`;
  await waitForReadiness(baseUrl);
  const loginResponse = await fetch(`${baseUrl}/api/auth/login`, {
    body: JSON.stringify({ username, password }),
    headers: { "content-type": "application/json" },
    method: "POST",
    signal: AbortSignal.timeout(10_000),
  });
  assertInvariant(loginResponse.ok, "smoke administrator login failed");
  const sessionCookie = loginResponse.headers
    .get("set-cookie")
    ?.split(";", 1)[0];
  assertInvariant(
    Boolean(sessionCookie),
    "smoke login omitted the session cookie",
  );
  const updateResponse = await fetch(`${baseUrl}/api/admin/runtime-settings`, {
    body: JSON.stringify({ values: { secure_cookies: "true" } }),
    headers: {
      "content-type": "application/json",
      cookie: sessionCookie,
    },
    method: "PUT",
    signal: AbortSignal.timeout(10_000),
  });
  assertInvariant(
    updateResponse.ok,
    "secure-cookie runtime setting update failed",
  );
  const removal = await removeManagedContainer(containerName);
  assertInvariant(
    removal.removed,
    `setup container cleanup failed: ${removal.error}`,
  );
  assertInvariant(await waitForPortClosure(), "setup listener remained open");
  hostPort = undefined;
}

/**
 * Execute the exact-image security and HTTP probes.
 *
 * @param {string} imageReference - Local tag or immutable registry digest reference.
 * @param {boolean} isDigestRequired - Whether a registry digest reference is mandatory.
 * @returns {Promise<Record<string, unknown>>} Successful smoke report before cleanup.
 */
async function runSmoke(imageReference, isDigestRequired) {
  const suffix = `${process.pid}-${randomBytes(4).toString("hex")}`;
  containerName = `litradar-smoke-${suffix}`;
  secretInitializerName = `${containerName}-secret-init`;
  volumeName = `litradar-smoke-data-${suffix}`;
  secretVolumeName = `litradar-smoke-secret-${suffix}`;

  const isDigestReference = DIGEST_REFERENCE_PATTERN.test(imageReference);
  assertInvariant(
    !isDigestRequired || isDigestReference,
    "release smoke requires a fully qualified image@sha256 digest reference",
  );
  if (isDigestReference) {
    await runDocker(["pull", imageReference], {
      timeoutMs: IMAGE_PULL_TIMEOUT_MS,
    });
  }
  const imageInspection = JSON.parse(
    (
      await runDocker([
        "image",
        "inspect",
        "--format",
        "{{json .}}",
        imageReference,
      ])
    ).stdout,
  );
  const imageId = imageInspection.Id;
  const repositoryDigests = imageInspection.RepoDigests ?? [];
  assertInvariant(
    typeof imageId === "string" && imageId.startsWith("sha256:"),
    "local image did not resolve to a content ID",
  );
  if (isDigestReference) {
    assertInvariant(
      repositoryDigests.some(
        (digest) => digest.toLowerCase() === imageReference.toLowerCase(),
      ),
      "pulled image metadata omitted the requested immutable digest",
    );
  }
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
  await enableSecureCookies(imageReference);
  await runDocker(buildServiceRunArguments(imageReference, true));

  hostPort = await resolvePublishedPort();
  const baseUrl = `http://127.0.0.1:${hostPort}`;
  await waitForReadiness(baseUrl);
  await waitForContainerHealth();

  await runDocker([
    "exec",
    containerName,
    "sh",
    "-c",
    "test -f /app/data/meta/ccf_computer_journals.csv && test -f /app/data/meta/chinese_journals.csv && test -f /app/data/meta/english_journals.csv",
  ]);
  await runDocker([
    "exec",
    containerName,
    "sh",
    "-c",
    'test "$(id -u)" = 10001 && test "$(id -g)" = 10001 && test "$(stat -c %a /run/secrets/litradar_key)" = 600 && test "$(stat -c %s /run/secrets/litradar_key)" = 32 && touch /app/data/smoke-write-probe && rm /app/data/smoke-write-probe && if touch /app/root-write-probe 2>/dev/null; then exit 1; fi && if printf x >> /run/secrets/litradar_key 2>/dev/null; then exit 1; fi && printf "#!/bin/sh\\nexit 0\\n" > /tmp/noexec-probe && chmod 700 /tmp/noexec-probe && if /tmp/noexec-probe 2>/dev/null; then exit 1; fi && rm /tmp/noexec-probe',
  ]);
  const [rootResponse, openApiResponse, authResponse, inspectResult] =
    await Promise.all([
      fetchRuntime(`${baseUrl}/`),
      fetchRuntime(`${baseUrl}/openapi.json`),
      fetchRuntime(`${baseUrl}/api/auth/me`),
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
  assertInvariant(
    authResponse.status === 401,
    `anonymous auth endpoint returned ${authResponse.status}`,
  );
  for (const response of [rootResponse, openApiResponse, authResponse]) {
    assertSecurityHeaders(response, true);
  }
  assertInvariant(
    rootResponse.headers.get("content-security-policy")?.includes("sha256-"),
    "root CSP omitted exported inline script hashes",
  );
  assertInvariant(
    authResponse.headers.get("cache-control") === "no-store" &&
      authResponse.headers.get("pragma") === "no-cache",
    "auth response was cacheable",
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
  const persistentMountDestinations = inspection.Mounts.map(
    (mount) => mount.Destination,
  ).sort();
  const writableMountDestinations = inspection.Mounts.filter(
    (mount) => mount.RW,
  ).map((mount) => mount.Destination);
  const temporaryFilesystemOptions = new Set(
    (inspection.HostConfig.Tmpfs?.["/tmp"] ?? "").split(","),
  );
  const portBindings = inspection.HostConfig.PortBindings ?? {};
  const publishedPorts = Object.keys(portBindings);
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
    !inspection.HostConfig.CapAdd || inspection.HostConfig.CapAdd.length === 0,
    "container adds Linux capabilities",
  );
  assertInvariant(
    inspection.Config.User === "10001:10001",
    "container does not declare the fixed unprivileged UID/GID",
  );
  assertInvariant(
    inspection.State.Health?.Status === "healthy",
    "Docker health state is not healthy",
  );
  assertInvariant(
    inspection.Config.Healthcheck?.Test?.[0] === "CMD-SHELL",
    "image does not define a Docker health check",
  );
  assertInvariant(
    inspection.Args.includes("--require-secure-cookies"),
    "hardened smoke did not require secure cookies",
  );
  assertInvariant(
    ["rw", "noexec", "nosuid", "nodev"].every((option) =>
      temporaryFilesystemOptions.has(option),
    ),
    "temporary filesystem omitted a required hardening option",
  );
  assertInvariant(
    publishedPorts.length === 1 && publishedPorts[0] === "8000/tcp",
    "container published an unexpected port",
  );
  assertInvariant(
    portBindings["8000/tcp"]?.length === 1 &&
      portBindings["8000/tcp"][0].HostIp === "127.0.0.1",
    "application port is not bound exclusively to host loopback",
  );
  assertInvariant(
    persistentMountDestinations.length === 2 &&
      persistentMountDestinations[0] === "/app/data" &&
      persistentMountDestinations[1] === "/run/secrets",
    "container has an unexpected persistent mount",
  );
  assertInvariant(
    dataMount?.Type === "volume" && dataMount?.Name === volumeName,
    "data mount is not the managed volume",
  );
  assertInvariant(
    writableMountDestinations.length === 1 &&
      writableMountDestinations[0] === "/app/data",
    "persistent write access is not limited to application data",
  );
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
    repositoryDigests,
    immutableDigestRequired: isDigestRequired,
    containerUser: inspection.Config.User,
    endpoints: ["/", "/health/ready", "/openapi.json", "/api/auth/me"],
    managedMetaPrepared: true,
    removedEnvironmentOverrides: [],
    security: {
      readOnlyRoot: true,
      capabilitiesDropped: true,
      noNewPrivileges: true,
      secureCookiesRequired: true,
      dockerHealthCheck: true,
      loopbackPublication: true,
      temporaryFilesystem: true,
      temporaryFilesystemOptions: ["rw", "noexec", "nosuid", "nodev"],
      writableDataVolume: true,
      readOnlySecretMount: true,
      responseHeaders: true,
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

const isDigestRequired = args[1] === "--require-digest";
if (
  args.length < 1 ||
  args.length > 2 ||
  !args[0].trim() ||
  (args.length === 2 && !isDigestRequired)
) {
  failure = new Error(
    "Usage: node scripts/container-smoke.mjs <image-reference> [--require-digest]",
  );
} else {
  try {
    report = await runSmoke(args[0].trim(), isDigestRequired);
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
  await fs.writeFile(FAILURE_LOG_PATH, FAILURE_LOG_MESSAGE, "utf8");
  process.stderr.write(`[container-smoke] ${safeFailure}\n`);
  process.exitCode =
    shutdownSignal === "SIGINT" ? 130 : shutdownSignal === "SIGTERM" ? 143 : 1;
} else {
  process.stdout.write(`${JSON.stringify(finalReport)}\n`);
}
