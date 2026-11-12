/** Regression checks for the full-stack supervisor's process ownership and bounded cleanup. */

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import net from 'node:net';
import { createInterface } from 'node:readline';
import test from 'node:test';

const MODULE_URL =
  process.env.LITRADAR_CLEANUP_MODULE ?? new URL('./full-stack-server.mjs', import.meta.url).href;
const { terminateProcessTree, waitForPortClosure } = await import(MODULE_URL);
const IS_UNIX = process.platform !== 'win32';

const LISTENER_SCRIPT = `
  const net = require('node:net');
  process.on('SIGTERM', () => {});
  const server = net.createServer(socket => socket.end());
  server.listen(0, '127.0.0.1', () => {
    process.send({ pid: process.pid, port: server.address().port });
  });
`;
const WRAPPER_SCRIPT = `
  const { spawn } = require('node:child_process');
  const child = spawn(process.execPath, ['-e', ${JSON.stringify(LISTENER_SCRIPT)}], {
    stdio: ['ignore', 'inherit', 'inherit', 'ipc'], windowsHide: true,
  });
  child.once('message', message => {
    process.stdout.write(JSON.stringify({ ...message, wrapperPid: process.pid }) + '\\n');
    if (process.argv.includes('exit-wrapper')) process.exit(0);
  });
  setInterval(() => {}, 1000);
`;

/** Start an owned wrapper with a listening descendant that ignores graceful termination. */
async function startWrapper(shouldExit = false) {
  const child = spawn(
    process.execPath,
    ['-e', WRAPPER_SCRIPT, ...(shouldExit ? ['exit-wrapper'] : [])],
    {
      detached: IS_UNIX,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  const output = createInterface({ input: child.stdout });
  const [line] = await once(output, 'line', { signal: AbortSignal.timeout(5000) });
  output.close();
  return { child, ...JSON.parse(line) };
}

/** Release only fixture processes started by this test, including after a failing assertion. */
function forceCleanup(fixture) {
  if (IS_UNIX) {
    try {
      process.kill(-fixture.wrapperPid, 'SIGKILL');
    } catch (error) {
      if (error.code !== 'ESRCH') throw error;
    }
  } else {
    for (const pid of [fixture.wrapperPid, fixture.pid]) {
      spawnSync('taskkill', ['/pid', String(pid), '/t', '/f'], {
        stdio: 'ignore',
        windowsHide: true,
        timeout: 5000,
      });
    }
  }
  fixture.child.stdout?.destroy();
  fixture.child.stderr?.destroy();
  fixture.child.unref();
}

/** Check that the descendant has stopped listening, without relying on the wrapper's exit. */
async function assertListenerClosed(port) {
  await waitForPortClosure(port, 'fixture descendant');
  const server = net.createServer();
  server.listen(port, '127.0.0.1');
  await once(server, 'listening');
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
}

void test(
  'terminates the listening descendant as well as its live wrapper',
  { timeout: 20000 },
  async () => {
    const fixture = await startWrapper();
    try {
      await terminateProcessTree(fixture.child);
      await assertListenerClosed(fixture.port);
    } finally {
      forceCleanup(fixture);
    }
  },
);

void test(
  'cleans the owned process group after its wrapper has already exited',
  {
    skip: !IS_UNIX,
    timeout: 20000,
  },
  async () => {
    const fixture = await startWrapper(true);
    try {
      if (fixture.child.exitCode === null) await once(fixture.child, 'exit');
      await terminateProcessTree(fixture.child);
      await assertListenerClosed(fixture.port);
    } finally {
      forceCleanup(fixture);
    }
  },
);

void test(
  'reports cleanup failure and exits despite inherited pipes held by a descendant',
  {
    skip: !IS_UNIX,
    timeout: 20000,
  },
  async () => {
    const script = `
    const { spawn } = await import('node:child_process');
    const { once } = await import('node:events');
    const { createInterface } = await import('node:readline');
    const { terminateProcessTree, waitForPortClosure } = await import(${JSON.stringify(MODULE_URL)});
    const child = spawn(process.execPath, ['-e', ${JSON.stringify(WRAPPER_SCRIPT)}], {
      detached: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    const lines = createInterface({ input: child.stdout });
    const [line] = await once(lines, 'line');
    lines.close();
    process.stdout.write(line + '\\n');
    process.kill = () => { throw Object.assign(new Error('injected cleanup denial'), { code: 'EACCES' }); };
    try {
      await terminateProcessTree(child);
      await waitForPortClosure(JSON.parse(line).port, 'fixture descendant');
    } catch (error) {
      process.stderr.write(error.message + '\\n');
      process.exitCode = 1;
    }
  `;
    const supervisor = spawn(process.execPath, ['--input-type=module', '-e', script], {
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let diagnostic = '';
    supervisor.stderr.setEncoding('utf8').on('data', (chunk) => {
      diagnostic += chunk;
    });
    let fixture;
    try {
      const lines = createInterface({ input: supervisor.stdout });
      const [line] = await once(lines, 'line', { signal: AbortSignal.timeout(5000) });
      lines.close();
      fixture = { child: supervisor, ...JSON.parse(line) };
      const [code] = await once(supervisor, 'exit', { signal: AbortSignal.timeout(8000) });
      assert.equal(code, 1);
      assert.match(diagnostic, /injected cleanup denial/);
    } finally {
      if (fixture) forceCleanup(fixture);
      forceCleanup({ child: supervisor, wrapperPid: supervisor.pid });
    }
  },
);
