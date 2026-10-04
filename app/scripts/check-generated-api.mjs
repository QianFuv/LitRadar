/** Generate the Go API contract in temporary files, then check or publish the formatted artifacts. */
import { execFileSync } from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as prettier from 'prettier';

const APP_ROOT = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const WORKSPACE_ROOT = path.dirname(APP_ROOT);
const ARGS = process.argv.slice(2);
if (ARGS.length > 1 || (ARGS.length === 1 && ARGS[0] !== '--write')) {
  throw new Error('Usage: node scripts/check-generated-api.mjs [--write]');
}
const SHOULD_WRITE = ARGS.length === 1;
const TEMP_ROOT = await fs.mkdtemp(path.join(os.tmpdir(), 'litradar-openapi-'));
try {
  const executable = path.join(
    TEMP_ROOT,
    process.platform === 'win32' ? 'litradar.exe' : 'litradar',
  );
  execFileSync(
    'go',
    [
      'build',
      '-mod=readonly',
      '-tags',
      'sqlite_fts5,sqlite_dbstat',
      '-o',
      executable,
      './cmd/litradar',
    ],
    {
      cwd: WORKSPACE_ROOT,
      env: { ...process.env, CGO_ENABLED: '1' },
      stdio: 'inherit',
      windowsHide: true,
      timeout: 300000,
    },
  );
  execFileSync(executable, ['openapi', '--output', path.join(TEMP_ROOT, 'openapi.json')], {
    cwd: TEMP_ROOT,
    stdio: 'inherit',
    windowsHide: true,
    timeout: 60000,
  });
  execFileSync(
    process.execPath,
    [
      path.join(APP_ROOT, 'node_modules/openapi-typescript/bin/cli.js'),
      path.join(TEMP_ROOT, 'openapi.json'),
      '-o',
      path.join(TEMP_ROOT, 'api-schema.tsx'),
      '--alphabetize',
    ],
    {
      cwd: APP_ROOT,
      stdio: 'inherit',
      windowsHide: true,
      timeout: 60000,
    },
  );
  const artifacts = [];
  for (const name of ['openapi.json', 'api-schema.tsx']) {
    const destination = path.join(APP_ROOT, 'lib/generated', name);
    const options = await prettier.resolveConfig(destination);
    const formatted = await prettier.format(await fs.readFile(path.join(TEMP_ROOT, name), 'utf8'), {
      ...options,
      filepath: destination,
    });
    artifacts.push({ destination, content: Buffer.from(formatted) });
  }
  for (const artifact of artifacts) {
    if (SHOULD_WRITE) {
      await fs.writeFile(artifact.destination, artifact.content);
    } else if (!(await fs.readFile(artifact.destination)).equals(artifact.content)) {
      console.error(
        `Generated API artifact changed: ${path.relative(APP_ROOT, artifact.destination)}`,
      );
      process.exitCode = 1;
    }
  }
} finally {
  if (path.dirname(TEMP_ROOT) !== path.resolve(os.tmpdir()))
    throw new Error('Generated API temporary path escaped its parent');
  await fs.rm(TEMP_ROOT, { recursive: true, force: true });
}
