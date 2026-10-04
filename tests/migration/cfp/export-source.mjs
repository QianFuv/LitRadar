/** Export source oracle observations only after all unchanged Rust assertions pass. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/cfp-oracle/source-build.json",
    "utf8",
  ),
);
const result = spawnSync(build.binary, ["--nocapture", "--test-threads=1"], {
  encoding: "utf8",
  windowsHide: true,
  timeout: 120000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(result.error);
await fs.writeFile(
  "output/migration/execution/cfp-oracle/source-original-tests.log",
  result.stdout + result.stderr,
);
assert.equal(result.status, 0, result.stdout + result.stderr);
const marker = "CFP_OBSERVATION:";
const cases = result.stdout
  .split(/\r?\n/)
  .filter((line) => line.includes(marker))
  .map((line) => JSON.parse(line.slice(line.indexOf(marker) + marker.length)));
assert(cases.length >= 70);
await fs.writeFile(
  "tests/migration/cfp/source-vectors.json",
  JSON.stringify(
    {
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/cfp/export-source.mjs"),
      ),
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log({ observations: cases.length });
