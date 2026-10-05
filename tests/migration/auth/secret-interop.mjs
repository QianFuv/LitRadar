/** Pass current synthetic Go envelopes through the independently frozen original Rust observer. */
import assert from "node:assert/strict";
import { verifyFrozenEvidence } from "../frozen-evidence.mjs";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Execute a bounded compiler or oracle and reject missing or partial results. */
function execute(executable, args, input) {
  const result = spawnSync(executable, args, {
    cwd: WORKSPACE_ROOT,
    shell: false,
    windowsHide: true,
    encoding: "utf8",
    input,
    timeout: 540_000,
    maxBuffer: 32 * 1024 * 1024,
    env: {
      ...process.env,
      GOWORK: "off",
      GOTOOLCHAIN: "go1.27.1",
      GOENV: "off",
      GOFLAGS: "",
    },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

await verifyFrozenEvidence();
const destination = "output/migration/execution/secret-oracle.exe";
const candidate = execute("go", [
  "run",
  "-mod=readonly",
  "-tags",
  "sqlite_fts5",
  "./tests/migration/auth/secret-candidate",
]);
const result = JSON.parse(execute(destination, [], candidate));
assert.equal(result.verified, 28);
const sources = [];
for (const filename of [
  "tests/migration/auth/secret-oracle.rs",
  "tests/migration/auth/secret-candidate/main.go",
  destination,
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "output/migration/execution/secret-interop-result.json",
  JSON.stringify(
    {
      result: "Passed",
      ...result,
      sources,
      observationsSha256: digest(candidate),
    },
    null,
    2,
  ) + "\n",
);
console.log(JSON.stringify(result));
