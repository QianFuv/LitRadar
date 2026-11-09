/**
 * Windows desktop smoke test for the native LitRadar client.
 *
 * Starts a disposable Go server seeded with the full-stack fixture, launches the smoke build
 * `apps/client/bin/litradar-client-smoke.exe` (a release build whose only addition is a
 * WebView2 remote debugging port read from `LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT`), and drives the real
 * WebView2 over CDP: server setup over loopback http, password login, search, favorite, citation
 * export through the system save dialog (filled with window messages), restart with the stored
 * token, and logout. Screenshots and the exported file go to
 * `apps/client/frontend/test-results/native-windows`.
 *
 * Usage, from apps/client/frontend after `node ../../server/scripts/build-go.mjs` and `task build:windows:smoke`:
 *   node tests/native-smoke/windows-smoke.mjs
 *
 * @packageDocumentation
 */

import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { expect } from '@playwright/test';

import {
  ARTICLE_TITLE,
  attachToAppPage,
  MEMBER_PASSWORD,
  MEMBER_USERNAME,
  delay,
  reservePort,
  startFixtureServer,
} from './support.mjs';

const execFileAsync = promisify(execFile);
const FRONTEND_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const CLIENT_ROOT = path.resolve(FRONTEND_ROOT, '..');
const CLIENT_BINARY = path.join(CLIENT_ROOT, 'bin', 'litradar-client-smoke.exe');
const RESULTS_DIR = path.join(FRONTEND_ROOT, 'test-results', 'native-windows');
const CONFIG_FILE = path.join(process.env.APPDATA ?? '', 'LitRadar', 'client.json');
const STEP_TIMEOUT_MS = 60_000;

/**
 * List Windows Credential Manager targets that belong to LitRadar.
 *
 * @returns Matching credential target lines.
 */
async function listLitRadarCredentials() {
  const { stdout } = await execFileAsync('cmdkey', ['/list']);
  return stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.includes('LitRadar/token/'));
}

/**
 * Launch the desktop client with WebView2 remote debugging and attach Playwright to it.
 *
 * @param debuggingPort - WebView2 remote debugging port.
 * @returns App process, CDP browser, and the app page.
 */
async function launchClient(debuggingPort) {
  const app = spawn(CLIENT_BINARY, [], {
    env: {
      ...process.env,
      LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT: String(debuggingPort),
    },
    stdio: 'ignore',
  });
  const attached = await attachToAppPage(
    `http://127.0.0.1:${debuggingPort}`,
    'http://wails.localhost',
    STEP_TIMEOUT_MS,
  );
  if (attached) {
    return { app, ...attached };
  }
  app.kill();
  throw new Error('could not attach to the WebView2 debugging endpoint');
}

/**
 * Close the CDP connection and terminate the desktop client.
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
 * Fill and confirm the Windows save dialog of the client process with window messages, which
 * need neither keyboard focus nor the clipboard.
 *
 * @param processId - Client process id.
 *
 * @param destination - Absolute file path to save to.
 */
async function completeSaveDialog(processId, destination) {
  const script = `
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class SaveDialogDriver {
  delegate bool EnumProc(IntPtr handle, IntPtr parameter);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr handle, StringBuilder name, int size);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr handle);
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr handle);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr handle, uint message, IntPtr wParam, string lParam);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr handle, uint message, IntPtr wParam, IntPtr lParam);
  static string ClassOf(IntPtr handle) { var name = new StringBuilder(256); GetClassName(handle, name, name.Capacity); return name.ToString(); }
  public static bool Save(uint processId, string path) {
    IntPtr dialog = IntPtr.Zero;
    EnumWindows((handle, parameter) => {
      uint owner; GetWindowThreadProcessId(handle, out owner);
      if (owner == processId && IsWindowVisible(handle) && ClassOf(handle) == "#32770") { dialog = handle; return false; }
      return true;
    }, IntPtr.Zero);
    if (dialog == IntPtr.Zero) { return false; }
    IntPtr nameField = IntPtr.Zero;
    EnumChildWindows(dialog, (handle, parameter) => {
      if (ClassOf(handle) == "Edit" && ClassOf(GetParent(handle)) == "ComboBox" && IsWindowVisible(handle)) { nameField = handle; return false; }
      return true;
    }, IntPtr.Zero);
    if (nameField == IntPtr.Zero) { return false; }
    SendMessage(nameField, 0x000C, IntPtr.Zero, path);
    SendMessage(dialog, 0x0111, (IntPtr)1, IntPtr.Zero);
    return true;
  }
}
'@
$deadline = (Get-Date).AddSeconds(90)
while (-not [SaveDialogDriver]::Save(${processId}, '${destination.replaceAll("'", "''")}')) {
  if ((Get-Date) -gt $deadline) { throw 'save dialog not found' }
  Start-Sleep -Milliseconds 300
}
`;
  await execFileAsync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    timeout: 2 * STEP_TIMEOUT_MS,
  });
}

/**
 * Wait until a file exists and return its text.
 *
 * @param filePath - File path.
 * @returns File text.
 */
async function waitForFile(filePath) {
  const deadline = Date.now() + STEP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const content = await fs.readFile(filePath, 'utf8').catch(() => null);
    if (content !== null) {
      return content;
    }
    await delay(250);
  }
  throw new Error(`file was not written: ${filePath}`);
}

/**
 * Run the end-to-end desktop journey.
 */
async function main() {
  await fs.mkdir(RESULTS_DIR, { recursive: true });
  const existingConfig = await fs.readFile(CONFIG_FILE, 'utf8').catch(() => null);
  if (existingConfig !== null) {
    throw new Error(`refusing to overwrite an existing client config at ${CONFIG_FILE}`);
  }
  const server = await startFixtureServer();
  const { origin } = server;
  const report = { origin, steps: [] };
  let client;
  try {
    const debuggingPort = await reservePort();
    client = await launchClient(debuggingPort);
    let { page } = client;
    page.setDefaultTimeout(STEP_TIMEOUT_MS);

    await page.getByLabel('服务器地址').fill(origin);
    await page.getByRole('button', { name: '连接', exact: true }).click();
    await expect(page.getByLabel('用户名或邮箱')).toBeVisible();
    await expect(page.getByRole('button', { name: /访问令牌/ })).toHaveCount(0);
    await page.screenshot({ path: path.join(RESULTS_DIR, '01-login.png') });
    report.steps.push('server setup');

    await page.getByLabel('用户名或邮箱').fill(MEMBER_USERNAME);
    await page.getByLabel('密码', { exact: true }).fill(MEMBER_PASSWORD);
    await page.getByRole('button', { name: '登录', exact: true }).click();
    const accountMenu = page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` });
    await expect(accountMenu).toBeVisible();
    report.credentialsAfterLogin = await listLitRadarCredentials();
    report.steps.push('password login');

    const searchInput = page.getByRole('combobox', { name: '搜索文章' });
    await searchInput.fill(ARTICLE_TITLE);
    await searchInput.press('Enter');
    await expect(page.getByText(ARTICLE_TITLE, { exact: true }).first()).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '02-search.png') });
    report.steps.push('search');

    await page
      .getByRole('button', { name: /^查看文章详情：/ })
      .first()
      .click();
    const reader = page.getByLabel('文章操作').filter({ visible: true }).first();
    await reader.getByRole('button', { name: '收藏', exact: true }).click();
    await page.getByRole('button', { name: 'Reading', exact: true }).click();
    await expect(reader.getByRole('button', { name: '已收藏', exact: true })).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '03-favorite.png') });
    report.steps.push('favorite add');

    await page.goto('http://wails.localhost/?view=favorites');
    await page
      .getByRole('button', { name: /Reading/ })
      .first()
      .click();
    await page.getByRole('button', { name: '导出收藏夹' }).click();
    const exportPath = path.join(RESULTS_DIR, `export-${Date.now()}.bib`);
    const dialogDone = completeSaveDialog(client.app.pid, exportPath);
    await page.getByRole('button', { name: '导出引用', exact: true }).click();
    await dialogDone;
    const exported = await waitForFile(exportPath);
    if (!exported.includes(ARTICLE_TITLE)) {
      throw new Error('exported citation file does not contain the favorite');
    }
    await expect(page.getByRole('status').filter({ hasText: /已导出/ })).toHaveCount(1, {
      timeout: STEP_TIMEOUT_MS,
    });
    await page.screenshot({ path: path.join(RESULTS_DIR, '04-export.png') });
    report.exportPath = exportPath;
    report.steps.push('export download saved');

    await closeClient(client);
    client = await launchClient(debuggingPort);
    page = client.page;
    page.setDefaultTimeout(STEP_TIMEOUT_MS);
    await expect(
      page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` }),
    ).toBeVisible();
    await page.screenshot({ path: path.join(RESULTS_DIR, '05-restart-signed-in.png') });
    report.steps.push('restart keeps session');

    await page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` }).click();
    await page.getByRole('menuitem', { name: '退出登录' }).click();
    await expect(page.getByLabel('用户名或邮箱')).toBeVisible();
    report.credentialsAfterLogout = await listLitRadarCredentials();
    if (report.credentialsAfterLogout.some((line) => line.includes(origin))) {
      throw new Error('token credential survived logout');
    }
    await page.screenshot({ path: path.join(RESULTS_DIR, '06-logout.png') });
    report.steps.push('logout');

    await page.getByRole('button', { name: '更换服务器' }).click();
    await expect(page.getByLabel('服务器地址')).toBeVisible();
    report.steps.push('switch server cleanup');
    report.status = 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
    await client?.page
      .screenshot({ path: path.join(RESULTS_DIR, 'failure.png') })
      .catch(() => undefined);
    report.labels = await client?.page
      .locator('[aria-label]')
      .evaluateAll((elements) =>
        elements.map((element) => `${element.tagName}:${element.getAttribute('aria-label')}`),
      )
      .catch(() => undefined);
  } finally {
    if (client) {
      await closeClient(client);
    }
    await server.stop();
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
