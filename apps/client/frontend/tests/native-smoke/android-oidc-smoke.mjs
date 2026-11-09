/**
 * Android single sign-on smoke test for the native LitRadar client on a running emulator.
 *
 * Starts a development-mode fixture server with OIDC pointed at the fake provider, maps both
 * ports into the device with `adb reverse`, and drives the app WebView and Chrome over their
 * forwarded DevTools sockets. Chrome is the real system browser: the app opens it through
 * `Intent.ACTION_VIEW`, and the server's final redirect to `litradar://oidc/callback` returns to
 * the app through its intent filter. Covers a sign-in whose app process the system killed
 * while Chrome was open (the login page must report the interruption) and a completed sign-in
 * that survives an app restart. Screenshots go to
 * `apps/client/frontend/test-results/native-android-oidc`.
 *
 * Usage, from apps/client/frontend after `node scripts/build-go.mjs` and installing a
 * debuggable APK of the current sources:
 *   node tests/native-smoke/android-oidc-smoke.mjs
 *
 * @packageDocumentation
 */

import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { expect } from '@playwright/test';

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
const RESULTS_DIR = path.join(FRONTEND_ROOT, 'test-results', 'native-android-oidc');
const PACKAGE_NAME = 'com.qianfuv.litradar';
const ACTIVITY = `${PACKAGE_NAME}/com.wails.app.MainActivity`;
const STEP_TIMEOUT_MS = 120_000;
const SSO_BUTTON_NAME = `使用 ${FAKE_OIDC_DISPLAY_NAME} 登录`;

/**
 * Run one adb command.
 *
 * @param args - adb arguments.
 * @returns Standard output.
 */
async function adb(...args) {
  const { stdout } = await execFileAsync('adb', args, {
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  });
  return stdout;
}

/**
 * Save a device screenshot.
 *
 * @param name - File name inside the results directory.
 */
async function captureDevice(name) {
  const { stdout } = await execFileAsync('adb', ['exec-out', 'screencap', '-p'], {
    encoding: 'buffer',
    maxBuffer: 64 * 1024 * 1024,
  });
  await fs.writeFile(path.join(RESULTS_DIR, name), stdout);
}

/**
 * Attach to the app WebView, starting the activity first when asked.
 *
 * @param devtoolsPort - Host port forwarded to the WebView DevTools socket.
 * @param shouldStart - Whether to start the activity before attaching.
 * @returns CDP browser and app page.
 */
async function attachApp(devtoolsPort, shouldStart) {
  if (shouldStart) {
    await adb('shell', 'am', 'start', '-W', '-n', ACTIVITY);
  }
  const deadline = Date.now() + STEP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const processId = (await adb('shell', 'pidof', PACKAGE_NAME).catch(() => '')).trim();
    if (processId) {
      await adb(
        'forward',
        `tcp:${devtoolsPort}`,
        `localabstract:webview_devtools_remote_${processId}`,
      ).catch(() => undefined);
      const attached = await attachToAppPage(
        `http://127.0.0.1:${devtoolsPort}`,
        'https://wails.localhost',
        10_000,
      );
      if (attached) {
        attached.page.setDefaultTimeout(STEP_TIMEOUT_MS);
        return attached;
      }
    }
    await delay(1_000);
  }
  throw new Error('could not attach to the app WebView');
}

/**
 * Attach to the Chrome tab that shows the fake provider. Each journey closes its provider tab,
 * because Chrome keeps a tab on the provider page after handing the callback to the app.
 *
 * @param chromePort - Host port forwarded to Chrome's DevTools socket.
 * @param issuer - Fake provider issuer URL.
 * @returns CDP browser and provider tab.
 */
async function attachProviderTab(chromePort, issuer) {
  await adb('forward', `tcp:${chromePort}`, 'localabstract:chrome_devtools_remote');
  const attached = await attachToAppPage(`http://127.0.0.1:${chromePort}`, issuer, STEP_TIMEOUT_MS);
  if (!attached) {
    throw new Error('Chrome did not open the identity provider');
  }
  attached.page.setDefaultTimeout(STEP_TIMEOUT_MS);
  return attached;
}

/**
 * Run the Android single sign-on journeys.
 */
async function main() {
  await fs.mkdir(RESULTS_DIR, { recursive: true });
  await adb('shell', 'pm', 'clear', PACKAGE_NAME);
  await adb('shell', 'am', 'force-stop', 'com.android.chrome');
  const provider = await startFakeOidcProvider();
  const server = await startFixtureServer({ isDevelopment: true });
  const { origin, port } = server;
  const providerPort = new URL(provider.issuer).port;
  const devtoolsPort = await reservePort();
  const chromePort = await reservePort();
  const report = { origin, issuer: provider.issuer, steps: [] };
  let app;
  let chrome;
  try {
    report.providers = await enableFixtureOidc(origin, {
      issuer: provider.issuer,
      clientId: FAKE_OIDC_CLIENT_ID,
      clientSecret: FAKE_OIDC_CLIENT_SECRET,
      displayName: FAKE_OIDC_DISPLAY_NAME,
    });
    await adb('reverse', `tcp:${port}`, `tcp:${port}`);
    await adb('reverse', `tcp:${providerPort}`, `tcp:${providerPort}`);

    app = await attachApp(devtoolsPort, true);
    await app.page.getByLabel('服务器地址').fill(origin);
    await app.page.getByRole('button', { name: '连接', exact: true }).click();
    await expect(app.page.getByRole('button', { name: SSO_BUTTON_NAME })).toBeVisible({
      timeout: STEP_TIMEOUT_MS,
    });
    await captureDevice('01-login-with-sso.png');
    report.steps.push('sso offered');

    await app.page.getByRole('button', { name: SSO_BUTTON_NAME }).click();
    chrome = await attachProviderTab(chromePort, provider.issuer);
    await expect(chrome.page.getByRole('heading', { name: 'Mock IdP sign-in' })).toBeVisible();
    await captureDevice('02-chrome-idp.png');
    await app.browser.close().catch(() => undefined);
    await adb('shell', 'am', 'kill', PACKAGE_NAME);
    await delay(2_000);
    report.processAfterKill = (await adb('shell', 'pidof', PACKAGE_NAME).catch(() => '')).trim();
    await chrome.page.getByRole('button', { name: 'Allow' }).click();
    await delay(3_000);
    await captureDevice('03-after-callback-to-killed-app.png');
    await chrome.page.close().catch(() => undefined);
    await chrome.browser.close().catch(() => undefined);
    app = await attachApp(devtoolsPort, false);
    await expect(app.page.getByRole('alert')).toContainText('单点登录被中断', {
      timeout: STEP_TIMEOUT_MS,
    });
    await captureDevice('04-interrupted.png');
    report.steps.push('killed app reports the interrupted sign-in');

    await app.page.getByRole('button', { name: SSO_BUTTON_NAME }).click();
    chrome = await attachProviderTab(chromePort, provider.issuer);
    await chrome.page.getByRole('button', { name: 'Allow' }).click();
    await delay(3_000);
    await chrome.page.close().catch(() => undefined);
    await chrome.browser.close().catch(() => undefined);
    const accountMenu = app.page.getByRole('button', {
      name: `打开账号菜单：${FAKE_OIDC_USERNAME}`,
    });
    await expect(accountMenu.first()).toBeAttached({ timeout: STEP_TIMEOUT_MS });
    await delay(1_000);
    await captureDevice('05-signed-in.png');
    report.steps.push('sso sign-in through Chrome');

    await app.browser.close().catch(() => undefined);
    await adb('shell', 'am', 'force-stop', PACKAGE_NAME);
    app = await attachApp(devtoolsPort, true);
    await expect(
      app.page.getByRole('button', { name: `打开账号菜单：${FAKE_OIDC_USERNAME}` }).first(),
    ).toBeAttached({ timeout: STEP_TIMEOUT_MS });
    await captureDevice('06-restart-signed-in.png');
    report.steps.push('restart keeps the sso session');
    report.idpAuthorizations = provider.authorizations;
    report.status = 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
    await captureDevice('failure.png').catch(() => undefined);
  } finally {
    await app?.browser.close().catch(() => undefined);
    await chrome?.browser.close().catch(() => undefined);
    await adb('forward', '--remove-all').catch(() => undefined);
    await adb('reverse', '--remove-all').catch(() => undefined);
    await adb('shell', 'am', 'force-stop', PACKAGE_NAME).catch(() => undefined);
    await server.stop();
    await provider.stop();
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
