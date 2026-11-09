/**
 * Windows single sign-on smoke test for the native LitRadar client.
 *
 * Starts a development-mode fixture server with OIDC pointed at the fake provider, launches
 * the smoke build of the desktop app over its WebView2 DevTools port, and plays the system
 * browser with a Playwright Chromium: the smoke build writes the URLs it would open to
 * `LITRADAR_CLIENT_SMOKE_BROWSER_URL_FILE` instead of starting the real default browser.
 * Covers a cancelled sign-in, a denied sign-in, and a completed sign-in that stores the token
 * in Credential Manager and survives an app restart. Screenshots go to
 * `apps/client/frontend/test-results/native-windows-oidc`.
 *
 * Usage, from apps/client/frontend after `node scripts/build-go.mjs` and building
 * `../bin/litradar-client-smoke.exe` (`task build:windows:smoke`):
 *   node tests/native-smoke/windows-oidc-smoke.mjs
 *
 * @packageDocumentation
 */

import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { chromium, expect } from '@playwright/test';

import {
  FAKE_OIDC_CLIENT_ID,
  FAKE_OIDC_CLIENT_SECRET,
  FAKE_OIDC_DISPLAY_NAME,
  FAKE_OIDC_USERNAME,
  startFakeOidcProvider,
} from './fake-oidc-provider.mjs';
import {
  attachToAppPage,
  delay,
  enableFixtureOidc,
  reservePort,
  startFixtureServer,
} from './support.mjs';

const execFileAsync = promisify(execFile);
const FRONTEND_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const CLIENT_ROOT = path.resolve(FRONTEND_ROOT, '..');
const CLIENT_BINARY = path.join(CLIENT_ROOT, 'bin', 'litradar-client-smoke.exe');
const RESULTS_DIR = path.join(FRONTEND_ROOT, 'test-results', 'native-windows-oidc');
const BROWSER_URL_FILE = path.join(RESULTS_DIR, 'browser-urls.txt');
const CONFIG_FILE = path.join(process.env.APPDATA ?? '', 'LitRadar', 'client.json');
const STEP_TIMEOUT_MS = 60_000;
const SSO_BUTTON_NAME = `使用 ${FAKE_OIDC_DISPLAY_NAME} 登录`;

/**
 * List the LitRadar token credentials in Windows Credential Manager.
 *
 * @returns Credential target lines.
 */
async function listLitRadarCredentials() {
  const { stdout } = await execFileAsync('cmdkey', ['/list']);
  return stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.includes('LitRadar/token/'));
}

/**
 * Launch the smoke build and attach to its WebView.
 *
 * @param debuggingPort - WebView2 remote debugging port.
 * @returns Process, CDP browser, and app page.
 */
async function launchClient(debuggingPort) {
  const app = spawn(CLIENT_BINARY, [], {
    env: {
      ...process.env,
      LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT: String(debuggingPort),
      LITRADAR_CLIENT_SMOKE_BROWSER_URL_FILE: BROWSER_URL_FILE,
    },
    stdio: 'ignore',
  });
  const attached = await attachToAppPage(
    `http://127.0.0.1:${debuggingPort}`,
    'http://wails.localhost',
    STEP_TIMEOUT_MS,
  );
  if (attached) {
    attached.page.setDefaultTimeout(STEP_TIMEOUT_MS);
    return { app, ...attached };
  }
  app.kill();
  throw new Error('could not attach to the WebView2 debugging endpoint');
}

/**
 * Stop the app process tree.
 *
 * @param client - Launched client.
 */
async function closeClient(client) {
  await client.browser.close().catch(() => undefined);
  await execFileAsync('taskkill', ['/PID', String(client.app.pid), '/T', '/F']).catch(
    () => undefined,
  );
  await delay(1_000);
}

/**
 * Wait for the next URL the app hands to the system browser.
 *
 * @param previousCount - Number of URLs already consumed.
 * @returns The new URL.
 */
async function waitForBrowserUrl(previousCount) {
  const deadline = Date.now() + STEP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const lines = (await fs.readFile(BROWSER_URL_FILE, 'utf8').catch(() => ''))
      .split(/\r?\n/)
      .filter(Boolean);
    if (lines.length > previousCount) {
      return lines[previousCount];
    }
    await delay(250);
  }
  throw new Error('the app did not open the system browser');
}

/**
 * Run the single sign-on journeys.
 */
async function main() {
  await fs.mkdir(RESULTS_DIR, { recursive: true });
  await fs.rm(BROWSER_URL_FILE, { force: true });
  const existingConfig = await fs.readFile(CONFIG_FILE, 'utf8').catch(() => null);
  if (existingConfig !== null) {
    throw new Error(`refusing to overwrite an existing client config at ${CONFIG_FILE}`);
  }
  const provider = await startFakeOidcProvider();
  const server = await startFixtureServer({ isDevelopment: true });
  const { origin } = server;
  const report = { origin, issuer: provider.issuer, steps: [] };
  let client;
  let systemBrowser;
  let openedUrls = 0;
  try {
    report.providers = await enableFixtureOidc(origin, {
      issuer: provider.issuer,
      clientId: FAKE_OIDC_CLIENT_ID,
      clientSecret: FAKE_OIDC_CLIENT_SECRET,
      displayName: FAKE_OIDC_DISPLAY_NAME,
    });
    systemBrowser = await chromium.launch();
    const debuggingPort = await reservePort();
    client = await launchClient(debuggingPort);
    let { page } = client;

    await page.getByLabel('服务器地址').fill(origin);
    await page.getByRole('button', { name: '连接', exact: true }).click();
    await expect(page.getByRole('button', { name: SSO_BUTTON_NAME })).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '01-login-with-sso.png') });
    report.steps.push('sso offered');

    await page.getByRole('button', { name: SSO_BUTTON_NAME }).click();
    await expect(page.getByRole('status').filter({ hasText: '已在浏览器中打开' })).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '02-waiting-for-browser.png') });
    const cancelledUrl = new URL(await waitForBrowserUrl(openedUrls));
    openedUrls += 1;
    await page.getByRole('button', { name: '取消单点登录' }).click();
    await expect(page.getByRole('button', { name: SSO_BUTTON_NAME })).toBeEnabled();
    const cancelledCallback = await fetch(
      `${cancelledUrl.searchParams.get('redirect_uri')}?code=x&state=${cancelledUrl.searchParams.get('state')}`,
    ).catch((error) => error);
    report.cancelledListenerClosed = cancelledCallback instanceof Error;
    if (!report.cancelledListenerClosed) {
      throw new Error('the loopback listener still answers after cancellation');
    }
    report.steps.push('cancel closes the loopback listener');

    await page.getByRole('button', { name: SSO_BUTTON_NAME }).click();
    const deniedUrl = await waitForBrowserUrl(openedUrls);
    openedUrls += 1;
    const deniedTab = await systemBrowser.newPage();
    await deniedTab.goto(deniedUrl);
    await deniedTab.getByRole('button', { name: 'Deny' }).click();
    await expect(deniedTab.getByRole('heading', { name: '登录未完成' })).toBeVisible();
    await deniedTab.screenshot({ path: path.join(RESULTS_DIR, '03-browser-denied.png') });
    await expect(page.getByRole('alert')).toContainText('授权已取消');
    await page.screenshot({ path: path.join(RESULTS_DIR, '04-app-denied.png') });
    await deniedTab.close();
    report.steps.push('denied sign-in reported');

    await page.getByRole('button', { name: SSO_BUTTON_NAME }).click();
    const loginUrl = new URL(await waitForBrowserUrl(openedUrls));
    openedUrls += 1;
    report.loginUrl = {
      path: loginUrl.pathname,
      redirectUri: loginUrl.searchParams.get('redirect_uri'),
      challengeLength: loginUrl.searchParams.get('code_challenge')?.length,
      stateLength: loginUrl.searchParams.get('state')?.length,
    };
    const browserTab = await systemBrowser.newPage();
    await browserTab.goto(loginUrl.href);
    await expect(browserTab.getByRole('heading', { name: 'Mock IdP sign-in' })).toBeVisible();
    await browserTab.screenshot({ path: path.join(RESULTS_DIR, '05-browser-idp.png') });
    await browserTab.getByRole('button', { name: 'Allow' }).click();
    await expect(browserTab.getByRole('heading', { name: '登录完成' })).toBeVisible();
    report.browserFinalUrl = browserTab.url().replace(/code=[^&]+/, 'code=<redacted>');
    await browserTab.screenshot({ path: path.join(RESULTS_DIR, '06-browser-complete.png') });

    const accountMenu = page.getByRole('button', { name: `打开账号菜单：${FAKE_OIDC_USERNAME}` });
    await expect(accountMenu).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '07-app-signed-in.png') });
    report.credentialsAfterLogin = await listLitRadarCredentials();
    if (!report.credentialsAfterLogin.some((line) => line.includes(origin))) {
      throw new Error('no Credential Manager token for the server after single sign-on');
    }
    report.steps.push('sso sign-in stores the token');

    await closeClient(client);
    client = await launchClient(debuggingPort);
    page = client.page;
    await expect(
      page.getByRole('button', { name: `打开账号菜单：${FAKE_OIDC_USERNAME}` }),
    ).toBeVisible();
    report.steps.push('restart keeps the sso session');

    await page.getByRole('button', { name: `打开账号菜单：${FAKE_OIDC_USERNAME}` }).click();
    await page.getByRole('menuitem', { name: '退出登录' }).click();
    await expect(page.getByRole('button', { name: SSO_BUTTON_NAME })).toBeVisible();
    report.credentialsAfterLogout = await listLitRadarCredentials();
    if (report.credentialsAfterLogout.some((line) => line.includes(origin))) {
      throw new Error('token credential survived logout');
    }
    report.steps.push('logout');
    report.idpAuthorizations = provider.authorizations;
    report.status = 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
    await client?.page
      .screenshot({ path: path.join(RESULTS_DIR, 'failure.png') })
      .catch(() => undefined);
  } finally {
    if (client) {
      await closeClient(client);
    }
    await systemBrowser?.close().catch(() => undefined);
    await server.stop();
    await provider.stop();
    await execFileAsync('cmdkey', [`/delete:LitRadar/token/${origin}`]).catch(() => undefined);
    await fs.rm(CONFIG_FILE, { force: true });
    await fs.writeFile(
      path.join(RESULTS_DIR, 'report.json'),
      `${JSON.stringify(report, null, 2)}\n`,
    );
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  }
  if (report.status !== 'passed') {
    process.exitCode = 1;
  }
}

await main();
