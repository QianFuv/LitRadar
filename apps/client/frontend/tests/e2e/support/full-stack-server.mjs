/**
 * Supervise one disposable LitRadar service and Vite preview for the real-backend browser suite.
 */

import { spawn } from 'node:child_process';
import { constants as fsConstants } from 'node:fs';
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const WORKSPACE_ROOT = path.resolve(APP_ROOT, '../../..');
const VP_CLI = path.join(APP_ROOT, 'node_modules', 'vite-plus', 'bin', 'vp');
const TARGET_ROOT = path.join(WORKSPACE_ROOT, 'target', 'go');
const EXECUTABLE_SUFFIX = process.platform === 'win32' ? '.exe' : '';
const SERVICE_BINARY = path.join(TARGET_ROOT, `litradar${EXECUTABLE_SUFFIX}`);
const SEEDER_BINARY = path.join(TARGET_ROOT, `litradar-fixture${EXECUTABLE_SUFFIX}`);
const MARKER_FILE = '.litradar-e2e-root';
const MARKER_CONTENT = 'litradar-full-stack-e2e-v1\n';
const READY_TIMEOUT_MS = 30_000;
const SHUTDOWN_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 100;

let fixtureRoot;
let serviceProcess;
let previewProcess;
let testProcess;
let shutdownSignal;
const terminationPromises = new WeakMap();

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
 * Replace the disposable root in diagnostics before forwarding them.
 *
 * @param {string} value - Raw diagnostic text.
 * @returns {string} Redacted diagnostic text.
 */
function sanitizeDiagnostic(value) {
  if (!fixtureRoot) {
    return value;
  }
  return value.split(fixtureRoot).join('<fixture-root>');
}

/**
 * Forward a child stream with a stable label and redacted temporary path.
 *
 * @param {import('node:stream').Readable | null} stream - Child output stream.
 * @param {string} label - Diagnostic source label.
 * @returns {void}
 */
function forwardDiagnostics(stream, label) {
  stream?.on('data', (chunk) => {
    process.stderr.write(`[${label}] ${sanitizeDiagnostic(String(chunk))}`);
  });
}

/**
 * Reject work promptly after an interrupt or termination request.
 *
 * @returns {void}
 */
function assertNotShuttingDown() {
  if (shutdownSignal) {
    throw new Error(`received ${shutdownSignal}`);
  }
}

/**
 * Resolve after a child process exits.
 *
 * @param {import('node:child_process').ChildProcess} child - Child process.
 * @returns {Promise<{code: number | null, signal: NodeJS.Signals | null}>} Exit details.
 */
function waitForExit(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve({ code: child.exitCode, signal: child.signalCode });
  }
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolve({ code, signal }));
  });
}

/**
 * Run one already-built helper and capture its output.
 *
 * @param {string} executable - Executable path.
 * @param {string[]} args - Process arguments.
 * @param {string} label - Failure label.
 * @returns {Promise<string>} Captured standard output.
 */
async function runCaptured(executable, args, label) {
  const child = spawn(executable, args, {
    cwd: WORKSPACE_ROOT,
    env: process.env,
    shell: false,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => {
    stdout += String(chunk);
  });
  child.stderr.on('data', (chunk) => {
    stderr += String(chunk);
  });
  const result = await waitForExit(child);
  if (result.code !== 0) {
    const diagnostic = sanitizeDiagnostic(stderr.trim());
    throw new Error(`${label} failed with exit code ${result.code}: ${diagnostic}`);
  }
  return stdout.trim();
}

/**
 * Check whether a server address is a bound TCP address rather than a pipe name or nothing.
 *
 * @param {ReturnType<net.Server['address']>} address - Address reported by the server.
 * @returns {address is net.AddressInfo} Whether the address has a port.
 */
function isPortAddress(address) {
  return address instanceof Object;
}

/**
 * Reserve and release an ephemeral loopback port.
 *
 * @returns {Promise<number>} Available TCP port.
 */
async function reserveLoopbackPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      if (!isPortAddress(address)) {
        server.close();
        reject(new Error('failed to reserve a loopback port'));
        return;
      }
      server.close((error) => {
        if (error) {
          reject(error);
        } else {
          resolve(address.port);
        }
      });
    });
  });
}

/**
 * Wait until one owned child's readiness URL succeeds or the bound expires.
 *
 * @param {string} url - Readiness URL.
 * @param {import('node:child_process').ChildProcess} child - Owned child serving the URL.
 * @param {string} label - Service label used in diagnostics.
 * @returns {Promise<void>} Promise resolved when the service is ready.
 */
async function waitForReadiness(url, child, label) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastError = 'readiness endpoint did not respond';
  while (Date.now() < deadline) {
    assertNotShuttingDown();
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`${label} exited before becoming ready`);
    }
    try {
      const response = await fetch(url, {
        signal: AbortSignal.timeout(1_000),
      });
      if (response.ok) {
        return;
      }
      lastError = `readiness endpoint returned ${response.status}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`${label} readiness timed out: ${lastError}`);
}

/**
 * Ask a child process tree to terminate, escalating after a bounded grace period.
 *
 * @param {import('node:child_process').ChildProcess | undefined} child - Child process.
 * @returns {Promise<void>} Promise resolved after the child exits.
 */
async function terminateProcessTree(child) {
  if (!child) {
    return;
  }
  const existing = terminationPromises.get(child);
  if (existing) {
    await existing;
    return;
  }
  const termination = terminateProcessTreeOnce(child);
  terminationPromises.set(child, termination);
  await termination;
}

/**
 * Perform one process-tree termination attempt.
 *
 * @param {import('node:child_process').ChildProcess} child - Owned process group leader.
 * @returns {Promise<void>} Promise resolved after termination handling.
 */
async function terminateProcessTreeOnce(child) {
  try {
    if (!child.pid) {
      return;
    }
    if (process.platform === 'win32') {
      if (child.exitCode !== null || child.signalCode !== null) {
        return;
      }
      const killer = spawn('taskkill', ['/pid', String(child.pid), '/t', '/f'], {
        shell: false,
        windowsHide: true,
        stdio: 'ignore',
      });
      try {
        if (!(await waitForExitWithin(killer, SHUTDOWN_TIMEOUT_MS))) {
          killer.kill();
          throw new Error(`taskkill timed out for process ${child.pid}`);
        }
      } finally {
        killer.unref();
      }
    } else {
      signalProcessGroup(child.pid, 'SIGTERM');
    }
    const hasExited = await waitForExitWithin(child, SHUTDOWN_TIMEOUT_MS);
    if (process.platform !== 'win32') {
      signalProcessGroup(child.pid, 'SIGKILL');
    } else if (!hasExited) {
      child.kill('SIGKILL');
    }
    if (!hasExited && !(await waitForExitWithin(child, SHUTDOWN_TIMEOUT_MS))) {
      throw new Error(`process ${child.pid} did not exit after termination`);
    }
  } finally {
    child.stdin?.destroy();
    child.stdout?.destroy();
    child.stderr?.destroy();
    child.unref();
  }
}

/**
 * Signal an owned Unix process group even after its leader has exited.
 *
 * @param {number} pid - Process group ID created with detached spawn.
 * @param {NodeJS.Signals} signal - Termination signal.
 * @returns {void}
 */
function signalProcessGroup(pid, signal) {
  try {
    process.kill(-pid, signal);
  } catch (error) {
    if (error.code !== 'ESRCH') {
      throw error;
    }
  }
}

/**
 * Bound a child exit wait and release its timer and listeners on every outcome.
 *
 * @param {import('node:child_process').ChildProcess} child - Child process.
 * @param {number} timeoutMs - Maximum wait in milliseconds.
 * @returns {Promise<boolean>} Whether the child exited before the deadline.
 */
async function waitForExitWithin(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return true;
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => finish(false), timeoutMs);
    const onExit = () => finish(true);
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const cleanup = () => {
      clearTimeout(timer);
      child.off('exit', onExit);
      child.off('error', onError);
    };
    const finish = (hasExited) => {
      cleanup();
      resolve(hasExited);
    };
    child.once('exit', onExit);
    child.once('error', onError);
  });
}

/**
 * Attempt every owned process cleanup before reporting any failure.
 *
 * @param {{port: number | undefined, label: string}[]} ports - Listeners to verify after termination.
 * @returns {Promise<void>} Promise resolved when all cleanup checks succeed.
 */
async function cleanupProcesses(ports = []) {
  const results = await Promise.allSettled(
    [testProcess, previewProcess, serviceProcess].map(terminateProcessTree),
  );
  results.push(
    ...(await Promise.allSettled(ports.map(({ port, label }) => waitForPortClosure(port, label)))),
  );
  const errors = results
    .filter((result) => result.status === 'rejected')
    .map((result) => result.reason);
  if (errors.length) {
    throw new AggregateError(errors, errors.map((error) => error.message).join('; '));
  }
}

/**
 * Determine whether a loopback port refuses new connections.
 *
 * @param {number} port - TCP port.
 * @returns {Promise<boolean>} True when no listener accepts the connection.
 */
async function isPortClosed(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(250);
    socket.once('connect', () => {
      socket.destroy();
      resolve(false);
    });
    socket.once('error', () => resolve(true));
    socket.once('timeout', () => {
      socket.destroy();
      resolve(true);
    });
  });
}

/**
 * Verify that the service listener closes within the cleanup bound.
 *
 * @param {number | undefined} port - Owned TCP port.
 * @param {string} label - Listener label used in diagnostics.
 * @returns {Promise<void>} Promise resolved after closure.
 */
async function waitForPortClosure(port, label = 'service') {
  if (!port) {
    return;
  }
  const deadline = Date.now() + SHUTDOWN_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await isPortClosed(port)) {
      return;
    }
    await delay(POLL_INTERVAL_MS);
  }
  throw new Error(`${label} port ${port} remained open after cleanup`);
}

/**
 * Remove only a correctly marked disposable root below the OS temporary directory.
 *
 * @returns {Promise<void>} Promise resolved after safe cleanup.
 */
async function removeFixtureRoot() {
  if (!fixtureRoot) {
    return;
  }
  const rootMetadata = await fs.lstat(fixtureRoot);
  if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
    throw new Error('refusing to remove a non-directory fixture root');
  }
  const [realRoot, realTemp] = await Promise.all([
    fs.realpath(fixtureRoot),
    fs.realpath(os.tmpdir()),
  ]);
  const relativeRoot = path.relative(realTemp, realRoot);
  if (!relativeRoot || relativeRoot.startsWith('..') || path.isAbsolute(relativeRoot)) {
    throw new Error('refusing to remove a fixture root outside the OS temporary directory');
  }
  const markerPath = path.join(realRoot, MARKER_FILE);
  const markerHandle = await fs.open(
    markerPath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  );
  try {
    const markerMetadata = await markerHandle.stat();
    if (!markerMetadata.isFile()) {
      throw new Error('refusing to remove a fixture root without a regular marker file');
    }
    const markerContent = await markerHandle.readFile('utf8');
    if (markerContent !== MARKER_CONTENT) {
      throw new Error('refusing to remove a fixture root with an invalid marker');
    }
  } finally {
    await markerHandle.close();
  }
  await fs.rm(realRoot, { recursive: true, force: false });
  fixtureRoot = undefined;
}

/**
 * Start the Vite preview server for the built client, proxying backend namespaces to Go.
 *
 * @param {number} port - Preview TCP port.
 * @param {string} backendUrl - Go service base URL.
 * @returns {import('node:child_process').ChildProcess} Preview child process.
 */
function startPreview(port, backendUrl) {
  const child = spawn(
    process.execPath,
    [VP_CLI, 'preview', '--host', '127.0.0.1', '--port', String(port), '--strictPort'],
    {
      cwd: APP_ROOT,
      env: { ...process.env, LITRADAR_BACKEND_URL: backendUrl },
      shell: false,
      detached: process.platform !== 'win32',
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  forwardDiagnostics(child.stderr, 'preview');
  return child;
}

/**
 * Run the locked Playwright CLI with the current Node.js executable.
 *
 * @param {string} baseUrl - Client preview base URL.
 * @param {string[]} playwrightArguments - Explicit Playwright CLI arguments.
 * @returns {Promise<number>} Playwright exit code.
 */
async function runPlaywright(baseUrl, playwrightArguments) {
  const playwrightScript = createRequire(import.meta.url).resolve('@playwright/test/cli');
  testProcess = spawn(
    process.execPath,
    [
      playwrightScript,
      'test',
      '--config',
      'playwright.full-stack.config.tsx',
      ...playwrightArguments,
    ],
    {
      cwd: APP_ROOT,
      env: {
        ...process.env,
        PLAYWRIGHT_FULL_STACK_BASE_URL: baseUrl,
        LITRADAR_CFP_FIXTURE_ROOT: fixtureRoot,
        LITRADAR_CFP_FIXTURE_SEEDER: SEEDER_BINARY,
      },
      shell: false,
      detached: process.platform !== 'win32',
      windowsHide: true,
      stdio: 'inherit',
    },
  );
  const result = await waitForExit(testProcess);
  if (result.signal) {
    throw new Error(`Playwright exited from signal ${result.signal}`);
  }
  return result.code ?? 1;
}

/**
 * Create, seed, serve, test, and clean one disposable full-stack environment.
 *
 * @param {string[]} playwrightArguments - Explicit Playwright CLI arguments.
 * @returns {Promise<number>} Process exit code.
 */
async function main(playwrightArguments) {
  await Promise.all([
    fs.access(SERVICE_BINARY, fsConstants.X_OK),
    fs.access(SEEDER_BINARY, fsConstants.X_OK),
  ]);
  assertNotShuttingDown();

  fixtureRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'litradar-full-stack-e2e-'));
  const secretKeyPath = path.join(fixtureRoot, 'secret.key');
  await Promise.all([
    fs.writeFile(path.join(fixtureRoot, MARKER_FILE), MARKER_CONTENT, { flag: 'wx' }),
    fs.writeFile(secretKeyPath, Buffer.alloc(32, 41), { flag: 'wx', mode: 0o600 }),
  ]);
  assertNotShuttingDown();

  const seedOutput = await runCaptured(
    SEEDER_BINARY,
    ['--project-root', fixtureRoot],
    'fixture seeder',
  );
  const seedReport = JSON.parse(seedOutput);
  if (seedReport.status !== 'seeded' || seedReport.article_count !== 2) {
    throw new Error('fixture seeder returned an unexpected report');
  }

  const port = await reserveLoopbackPort();
  const baseUrl = `http://127.0.0.1:${port}`;
  serviceProcess = spawn(
    SERVICE_BINARY,
    [
      'serve',
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--project-root',
      fixtureRoot,
      '--secret-key-file',
      secretKeyPath,
      '--scheduler-interval-seconds',
      '3600',
    ],
    {
      cwd: WORKSPACE_ROOT,
      env: process.env,
      shell: false,
      detached: process.platform !== 'win32',
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  forwardDiagnostics(serviceProcess.stdout, 'backend');
  forwardDiagnostics(serviceProcess.stderr, 'backend');

  let previewPort;
  try {
    await waitForReadiness(`${baseUrl}/health/ready`, serviceProcess, 'Go service');
    process.stdout.write(`[full-stack] Go service ready at ${baseUrl}\n`);
    previewPort = await reserveLoopbackPort();
    const previewUrl = `http://127.0.0.1:${previewPort}`;
    previewProcess = startPreview(previewPort, baseUrl);
    await waitForReadiness(`${previewUrl}/login`, previewProcess, 'Vite preview');
    process.stdout.write(`[full-stack] Client preview ready at ${previewUrl}\n`);
    return await runPlaywright(previewUrl, playwrightArguments);
  } finally {
    await cleanupProcesses([
      { port: previewPort, label: 'Vite preview' },
      { port, label: 'Go service' },
    ]);
  }
}

export { terminateProcessTree, waitForPortClosure };

if (import.meta.main) {
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.once(signal, () => {
      shutdownSignal = signal;
      void cleanupProcesses().catch(() => undefined);
    });
  }

  try {
    const playwrightArguments = process.argv.slice(process.argv[2] === '--' ? 3 : 2);
    process.exitCode = await main(playwrightArguments);
  } catch (error) {
    process.stderr.write(
      `[full-stack] ${sanitizeDiagnostic(error instanceof Error ? (error.stack ?? error.message) : String(error))}\n`,
    );
    process.exitCode = shutdownSignal === 'SIGINT' ? 130 : shutdownSignal === 'SIGTERM' ? 143 : 1;
  } finally {
    try {
      try {
        await cleanupProcesses();
      } finally {
        await removeFixtureRoot();
      }
    } catch (error) {
      process.stderr.write(
        `[full-stack] cleanup failed: ${sanitizeDiagnostic(error instanceof Error ? error.message : String(error))}\n`,
      );
      process.exitCode = 1;
    }
  }
}
