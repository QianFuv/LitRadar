/**
 * Shared support for the native smoke drivers: a disposable LitRadar Go server seeded with the
 * full-stack fixture and attachment to an app WebView over CDP.
 *
 * @packageDocumentation
 */

import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { chromium } from '@playwright/test';

const execFileAsync = promisify(execFile);
const WORKSPACE_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..');
const EXECUTABLE_SUFFIX = process.platform === 'win32' ? '.exe' : '';
const SERVICE_BINARY = path.join(WORKSPACE_ROOT, 'target', 'go', `litradar${EXECUTABLE_SUFFIX}`);
const SEEDER_BINARY = path.join(
  WORKSPACE_ROOT,
  'target',
  'go',
  `litradar-fixture${EXECUTABLE_SUFFIX}`,
);
const READY_TIMEOUT_MS = 60_000;

/** Seeded member account of the full-stack fixture. */
export const MEMBER_USERNAME = 'fullstack_member';
/** Password of {@link MEMBER_USERNAME}. */
export const MEMBER_PASSWORD = 'FullStackMember!2026';
/** Title of the seeded article used by the journeys. */
export const ARTICLE_TITLE = 'Evidence Graphs for Living Literature Reviews';

/**
 * Wait for a duration.
 *
 * @param durationMs - Delay in milliseconds.
 * @returns Promise resolved after the delay.
 */
export function delay(durationMs) {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

/**
 * Reserve a free loopback port.
 *
 * @returns Port number.
 */
export function reservePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

/**
 * Poll a URL until it answers 200.
 *
 * @param url - Readiness URL.
 */
async function waitForReady(url) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const response = await fetch(url, { signal: AbortSignal.timeout(2_000) }).catch(() => null);
    if (response?.ok) {
      return;
    }
    await delay(250);
  }
  throw new Error(`timed out waiting for ${url}`);
}

/**
 * Seed a temporary project root and serve it on a loopback port.
 *
 * @returns Server origin, port, and an idempotent stop function.
 */
export async function startFixtureServer() {
  const fixtureRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'litradar-native-smoke-'));
  const secretKey = path.join(fixtureRoot, 'secret.key');
  await fs.writeFile(secretKey, Buffer.alloc(32, 41));
  await fs.writeFile(path.join(fixtureRoot, '.litradar-e2e-root'), 'litradar-full-stack-e2e-v1\n');
  await execFileAsync(SEEDER_BINARY, ['--project-root', fixtureRoot]);
  const port = await reservePort();
  const origin = `http://127.0.0.1:${port}`;
  const server = spawn(
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
      secretKey,
      '--scheduler-interval-seconds',
      '3600',
    ],
    { stdio: 'ignore' },
  );

  /**
   * Stop the server and remove its project root.
   */
  async function stop() {
    server.kill();
    await delay(500);
    await fs.rm(fixtureRoot, { recursive: true, force: true }).catch(() => undefined);
  }

  try {
    await waitForReady(`${origin}/health/ready`);
  } catch (error) {
    await stop();
    throw error;
  }
  return { origin, port, stop };
}

/**
 * Attach Playwright to the app page behind a CDP endpoint, retrying until it appears.
 *
 * @param endpoint - CDP HTTP endpoint.
 * @param urlPrefix - Origin prefix of the app page.
 * @param timeoutMs - Attachment deadline.
 * @returns CDP browser and app page, or null after the deadline.
 */
export async function attachToAppPage(endpoint, urlPrefix, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const browser = await chromium.connectOverCDP(endpoint).catch(() => null);
    const page = browser
      ?.contexts()
      .flatMap((context) => context.pages())
      .find((candidate) => candidate.url().startsWith(urlPrefix));
    if (page) {
      return { browser, page };
    }
    await browser?.close();
    await delay(500);
  }
  return null;
}
