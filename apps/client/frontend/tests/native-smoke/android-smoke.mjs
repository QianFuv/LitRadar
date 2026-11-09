/**
 * Android smoke test for the native LitRadar client on a running emulator or device.
 *
 * Starts a disposable Go server seeded with the full-stack fixture on host loopback, maps the
 * same port into the device with `adb reverse` so the app connects to `http://127.0.0.1:<port>`
 * (a loopback origin the host bridge accepts without any cleartext exception), launches the
 * installed debuggable APK, and drives its WebView over the forwarded DevTools socket: server
 * setup, password login, search, favorite, citation export into the public Downloads
 * collection, and an app restart that keeps the session. Screenshots go to
 * `apps/client/frontend/test-results/native-android`.
 *
 * Usage, from apps/client/frontend after `node ../../server/scripts/build-go.mjs`,
 * `task build:android:debug`, and `adb install -r ../bin/litradar-client_<version>_android_arm64_debug.apk`:
 *   node tests/native-smoke/android-smoke.mjs
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
  ARTICLE_TITLE,
  MEMBER_PASSWORD,
  MEMBER_USERNAME,
  attachToAppPage,
  delay,
  reservePort,
  startFixtureServer,
} from './support.mjs';

const execFileAsync = promisify(execFile);
const FRONTEND_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const RESULTS_DIR = path.join(FRONTEND_ROOT, 'test-results', 'native-android');
const PACKAGE_NAME = 'com.qianfuv.litradar';
const ACTIVITY = `${PACKAGE_NAME}/com.wails.app.MainActivity`;
const STEP_TIMEOUT_MS = 120_000;

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
 * Start the app and attach to its WebView through the DevTools socket of its process.
 *
 * @param devtoolsPort - Host port forwarded to the WebView DevTools socket.
 * @returns CDP browser and app page.
 */
async function launchApp(devtoolsPort) {
  await adb('shell', 'am', 'start', '-W', '-n', ACTIVITY);
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
        return attached;
      }
    }
    await delay(1_000);
  }
  throw new Error('could not attach to the app WebView');
}

/**
 * Stop the app and drop the DevTools forward.
 *
 * @param attached - Attached browser.
 * @param devtoolsPort - Forwarded host port.
 */
async function stopApp(attached, devtoolsPort) {
  await attached?.browser.close().catch(() => undefined);
  await adb('forward', '--remove', `tcp:${devtoolsPort}`).catch(() => undefined);
  await adb('shell', 'am', 'force-stop', PACKAGE_NAME).catch(() => undefined);
}

/**
 * Run the end-to-end Android journey.
 */
async function main() {
  await fs.mkdir(RESULTS_DIR, { recursive: true });
  await adb('shell', 'pm', 'clear', PACKAGE_NAME);
  await adb('shell', 'rm', '-f', '/sdcard/Download/Reading*.bib');
  const server = await startFixtureServer();
  const { origin, port } = server;
  const devtoolsPort = await reservePort();
  const report = { origin, steps: [] };
  let attached;
  try {
    await adb('reverse', `tcp:${port}`, `tcp:${port}`);
    report.abi = (await adb('shell', 'getprop', 'ro.product.cpu.abilist')).trim();
    attached = await launchApp(devtoolsPort);
    let { page } = attached;
    page.setDefaultTimeout(STEP_TIMEOUT_MS);

    await page.getByLabel('服务器地址').fill(origin);
    await page.getByRole('button', { name: '连接', exact: true }).click();
    await expect(page.getByLabel('用户名或邮箱')).toBeVisible({ timeout: STEP_TIMEOUT_MS });
    await captureDevice('01-login.png');
    report.steps.push('server setup');

    await page.getByLabel('用户名或邮箱').fill(MEMBER_USERNAME);
    await page.getByLabel('密码', { exact: true }).fill(MEMBER_PASSWORD);
    await page.getByRole('button', { name: '登录', exact: true }).click();
    const accountMenu = page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` });
    await expect(accountMenu.first()).toBeAttached({ timeout: STEP_TIMEOUT_MS });
    report.steps.push('password login');

    const searchInput = page.getByRole('combobox', { name: '搜索文章' });
    await searchInput.fill(ARTICLE_TITLE);
    await searchInput.press('Enter');
    await expect(page.getByText(ARTICLE_TITLE, { exact: true }).first()).toBeVisible({
      timeout: STEP_TIMEOUT_MS,
    });
    await captureDevice('02-search.png');
    report.steps.push('search');

    await page
      .getByRole('button', { name: /^查看文章详情：/ })
      .first()
      .click();
    const actions = page.getByLabel('文章操作').filter({ visible: true }).first();
    await actions.getByRole('button', { name: '收藏', exact: true }).click();
    await page.getByRole('button', { name: 'Reading', exact: true }).click();
    await expect(actions.getByRole('button', { name: '已收藏', exact: true })).toBeVisible({
      timeout: STEP_TIMEOUT_MS,
    });
    report.steps.push('favorite add');

    await page.goto('https://wails.localhost/?view=favorites');
    await page
      .getByRole('button', { name: /Reading/ })
      .first()
      .click();
    await page.getByRole('button', { name: '导出收藏夹' }).click();
    await page.getByRole('button', { name: '导出引用', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: /已导出/ })).toHaveCount(1, {
      timeout: STEP_TIMEOUT_MS,
    });
    await captureDevice('03-export.png');
    const downloads = await adb('shell', 'ls', '-l', '/sdcard/Download/');
    report.downloads = downloads
      .split(/\r?\n/)
      .filter((line) => line.includes('.bib'))
      .map((line) => line.trim());
    const exported = await adb('shell', 'cat', '/sdcard/Download/Reading.bib');
    if (!exported.includes(ARTICLE_TITLE)) {
      throw new Error('Downloads/Reading.bib does not contain the favorite');
    }
    report.steps.push('export saved to Downloads');

    await stopApp(attached, devtoolsPort);
    attached = await launchApp(devtoolsPort);
    page = attached.page;
    await expect(
      page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` }).first(),
    ).toBeAttached({ timeout: STEP_TIMEOUT_MS });
    await captureDevice('04-restart-signed-in.png');
    report.steps.push('restart keeps session');
    report.status = 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
    await captureDevice('failure.png').catch(() => undefined);
  } finally {
    await stopApp(attached, devtoolsPort);
    await adb('reverse', '--remove', `tcp:${port}`).catch(() => undefined);
    await server.stop();
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
