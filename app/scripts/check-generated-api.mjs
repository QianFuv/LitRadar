/**
 * Verify that regenerating the frontend API contract is idempotent.
 */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const GENERATED_FILE_PATHS = ['lib/generated/openapi.json', 'lib/generated/api-schema.tsx'];
const originalFiles = new Map();
for (const filePath of GENERATED_FILE_PATHS) {
  originalFiles.set(filePath, readFileSync(filePath));
}

const packageManagerExecutable = process.env.npm_execpath;
if (!packageManagerExecutable) {
  throw new Error('npm_execpath is required to regenerate the API contract');
}
execFileSync(process.execPath, [packageManagerExecutable, 'generate:api'], {
  stdio: 'inherit',
});

let hasChangedFile = false;
for (const filePath of GENERATED_FILE_PATHS) {
  if (!originalFiles.get(filePath).equals(readFileSync(filePath))) {
    console.error(`Generated API artifact changed: ${filePath}`);
    hasChangedFile = true;
  }
}
if (hasChangedFile) {
  process.exitCode = 1;
}
