/**
 * Generate the Go API contract in temporary files, then check or publish the formatted artifacts.
 *
 * openapi-typescript builds its output with the TypeScript 5/6 compiler API, which the
 * TypeScript 7 package no longer ships, so its `typescript` import resolves to the
 * `@typescript/typescript6` API package instead.
 */
import { execFileSync } from 'node:child_process';
import fs from 'node:fs/promises';
import { registerHooks } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const APP_ROOT = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const WORKSPACE_ROOT = path.resolve(APP_ROOT, '../../..');
const VP_BIN = path.join(APP_ROOT, 'node_modules/vite-plus/bin/vp');
const ARGS = process.argv.slice(2);
if (ARGS.length > 1 || (ARGS.length === 1 && ARGS[0] !== '--write')) {
  throw new Error('Usage: node scripts/check-generated-api.mjs [--write]');
}
const SHOULD_WRITE = ARGS.length === 1;

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === 'typescript' && context.parentURL?.includes('/openapi-typescript/')) {
      return nextResolve('@typescript/typescript6', { ...context, parentURL: import.meta.url });
    }
    return nextResolve(specifier, context);
  },
});
const { default: openapiTS, astToString, COMMENT_HEADER } = await import('openapi-typescript');

/**
 * Format generated source with the project's Oxfmt settings.
 *
 * @param {string} destination - Repository path the source is published to.
 * @param {string} source - Unformatted source text.
 * @returns {Buffer} Formatted source.
 */
function formatArtifact(destination, source) {
  return execFileSync(
    process.execPath,
    [VP_BIN, 'fmt', '--stdin-filepath', path.relative(APP_ROOT, destination)],
    {
      cwd: APP_ROOT,
      input: source,
      maxBuffer: 64 * 1024 * 1024,
      windowsHide: true,
      timeout: 60000,
    },
  );
}

const TEMP_ROOT = await fs.mkdtemp(path.join(os.tmpdir(), 'litradar-openapi-'));
if (path.dirname(TEMP_ROOT) !== path.resolve(os.tmpdir())) {
  throw new Error('Generated API temporary path escaped its parent');
}
try {
  const executable = path.join(
    TEMP_ROOT,
    process.platform === 'win32' ? 'litradar.exe' : 'litradar',
  );
  const openapiPath = path.join(TEMP_ROOT, 'openapi.json');
  execFileSync('go', ['build', '-mod=readonly', '-o', executable, './apps/server'], {
    cwd: WORKSPACE_ROOT,
    env: { ...process.env, CGO_ENABLED: '0' },
    stdio: 'inherit',
    windowsHide: true,
    timeout: 300000,
  });
  execFileSync(executable, ['openapi', '--output', openapiPath], {
    cwd: TEMP_ROOT,
    stdio: 'inherit',
    windowsHide: true,
    timeout: 60000,
  });
  const schemaSource = `${COMMENT_HEADER}${astToString(
    await openapiTS(pathToFileURL(openapiPath), { alphabetize: true, silent: true }),
  )}`;
  const sources = {
    'openapi.json': await fs.readFile(openapiPath, 'utf8'),
    'api-schema.tsx': schemaSource,
  };
  const artifacts = Object.entries(sources).map(([name, source]) => {
    const destination = path.join(APP_ROOT, 'lib/generated', name);
    return { destination, content: formatArtifact(destination, source) };
  });
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
  await fs.rm(TEMP_ROOT, { recursive: true, force: true });
}
