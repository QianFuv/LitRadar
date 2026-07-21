/**
 * Supervise one disposable LitRadar service for the real-backend browser suite.
 */

import { spawn } from 'node:child_process';
import { constants as fsConstants } from 'node:fs';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const WORKSPACE_ROOT = path.resolve(APP_ROOT, '..');
const TARGET_ROOT = path.join(WORKSPACE_ROOT, 'target', 'debug');
const EXECUTABLE_SUFFIX = process.platform === 'win32' ? '.exe' : '';
const SERVICE_BINARY = path.join(TARGET_ROOT, `litradar${EXECUTABLE_SUFFIX}`);
const SEEDER_BINARY = path.join(TARGET_ROOT, 'examples', `full_stack_fixture${EXECUTABLE_SUFFIX}`);
const MARKER_FILE = '.litradar-e2e-root';
const MARKER_CONTENT = 'litradar-full-stack-e2e-v1\n';
const READY_TIMEOUT_MS = 30_000;
const SHUTDOWN_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 100;

let fixtureRoot;
let serviceProcess;
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
      if (!address || typeof address === 'string') {
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
 * Wait until the Rust readiness endpoint succeeds or the bound expires.
 *
 * @param {string} baseUrl - Service base URL.
 * @returns {Promise<void>} Promise resolved when the service is ready.
 */
async function waitForReadiness(baseUrl) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastError = 'readiness endpoint did not respond';
  while (Date.now() < deadline) {
    assertNotShuttingDown();
    if (serviceProcess.exitCode !== null || serviceProcess.signalCode !== null) {
      throw new Error('Rust service exited before becoming ready');
    }
    try {
      const response = await fetch(`${baseUrl}/health/ready`, {
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
  throw new Error(`Rust service readiness timed out: ${lastError}`);
}

/**
 * Ask a child process tree to terminate, escalating after a bounded grace period.
 *
 * @param {import('node:child_process').ChildProcess | undefined} child - Child process.
 * @returns {Promise<void>} Promise resolved after the child exits.
 */
async function terminateProcessTree(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
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
 * @param {import('node:child_process').ChildProcess} child - Live child process.
 * @returns {Promise<void>} Promise resolved after termination handling.
 */
async function terminateProcessTreeOnce(child) {
  if (process.platform === 'win32' && child.pid) {
    const killer = spawn('taskkill', ['/pid', String(child.pid), '/t', '/f'], {
      shell: false,
      stdio: 'ignore',
    });
    await waitForExit(killer).catch(() => undefined);
  } else {
    child.kill('SIGTERM');
  }
  const gracefulExit = await Promise.race([
    waitForExit(child).then(() => true),
    delay(SHUTDOWN_TIMEOUT_MS).then(() => false),
  ]).catch(() => true);
  if (!gracefulExit && child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL');
    await waitForExit(child).catch(() => undefined);
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
 * @param {number | undefined} port - Service TCP port.
 * @returns {Promise<void>} Promise resolved after closure.
 */
async function waitForPortClosure(port) {
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
  throw new Error(`service port ${port} remained open after cleanup`);
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
  const markerMetadata = await fs.lstat(markerPath);
  if (!markerMetadata.isFile() || markerMetadata.isSymbolicLink()) {
    throw new Error('refusing to remove a fixture root without a regular marker file');
  }
  const markerContent = await fs.readFile(markerPath, 'utf8');
  if (markerContent !== MARKER_CONTENT) {
    throw new Error('refusing to remove a fixture root with an invalid marker');
  }
  await fs.rm(realRoot, { recursive: true, force: false });
  fixtureRoot = undefined;
}

/**
 * Run Playwright through the package manager executable used for this script.
 *
 * @param {string} baseUrl - Rust service base URL.
 * @returns {Promise<number>} Playwright exit code.
 */
async function runPlaywright(baseUrl) {
  const packageManagerScript = process.env.npm_execpath;
  if (!packageManagerScript) {
    throw new Error('npm_execpath is required to launch Playwright');
  }
  testProcess = spawn(
    process.execPath,
    [
      packageManagerScript,
      'exec',
      'playwright',
      'test',
      '--config',
      'playwright.full-stack.config.tsx',
    ],
    {
      cwd: APP_ROOT,
      env: { ...process.env, PLAYWRIGHT_FULL_STACK_BASE_URL: baseUrl },
      shell: false,
      stdio: 'inherit',
    },
  );
  const result = await waitForExit(testProcess);
  testProcess = undefined;
  if (result.signal) {
    throw new Error(`Playwright exited from signal ${result.signal}`);
  }
  return result.code ?? 1;
}

/**
 * Create, seed, serve, test, and clean one disposable full-stack environment.
 *
 * @returns {Promise<number>} Process exit code.
 */
async function main() {
  await Promise.all([
    fs.access(path.join(APP_ROOT, 'out'), fsConstants.R_OK),
    fs.access(SERVICE_BINARY, fsConstants.X_OK),
    fs.access(SEEDER_BINARY, fsConstants.X_OK),
  ]);
  assertNotShuttingDown();

  fixtureRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'litradar-full-stack-e2e-'));
  const secretKeyPath = path.join(fixtureRoot, 'secret.key');
  await Promise.all([
    fs.writeFile(path.join(fixtureRoot, MARKER_FILE), MARKER_CONTENT, { flag: 'wx' }),
    fs.writeFile(secretKeyPath, Buffer.alloc(32, 41), { flag: 'wx', mode: 0o600 }),
    fs.cp(path.join(APP_ROOT, 'out'), path.join(fixtureRoot, 'web'), { recursive: true }),
  ]);
  assertNotShuttingDown();

  const seedOutput = await runCaptured(
    SEEDER_BINARY,
    ['--project-root', fixtureRoot],
    'fixture seeder',
  );
  const seedReport = JSON.parse(seedOutput);
  if (seedReport.status !== 'seeded' || seedReport.article_count !== 1) {
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
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  forwardDiagnostics(serviceProcess.stdout, 'backend');
  forwardDiagnostics(serviceProcess.stderr, 'backend');

  try {
    await waitForReadiness(baseUrl);
    process.stdout.write(`[full-stack] Rust service ready at ${baseUrl}\n`);
    return await runPlaywright(baseUrl);
  } finally {
    await terminateProcessTree(testProcess);
    await terminateProcessTree(serviceProcess);
    serviceProcess = undefined;
    await waitForPortClosure(port);
  }
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => {
    shutdownSignal = signal;
    void terminateProcessTree(testProcess);
    void terminateProcessTree(serviceProcess);
  });
}

try {
  process.exitCode = await main();
} catch (error) {
  process.stderr.write(
    `[full-stack] ${sanitizeDiagnostic(error instanceof Error ? (error.stack ?? error.message) : String(error))}\n`,
  );
  process.exitCode = shutdownSignal === 'SIGINT' ? 130 : shutdownSignal === 'SIGTERM' ? 143 : 1;
} finally {
  try {
    await terminateProcessTree(testProcess);
    await terminateProcessTree(serviceProcess);
    await removeFixtureRoot();
  } catch (error) {
    process.stderr.write(
      `[full-stack] cleanup failed: ${sanitizeDiagnostic(error instanceof Error ? error.message : String(error))}\n`,
    );
    process.exitCode = 1;
  }
}
