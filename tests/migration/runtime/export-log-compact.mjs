/** Freeze compact formatting from the independently compiled original dependency observer. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { BASELINE, digest } from "../oracle.mjs";

const executable = "output/migration/execution/logging-oracle.exe";
const result = spawnSync(executable, [], {
  input: JSON.stringify({ compact: true }),
  encoding: "utf8",
  windowsHide: true,
  timeout: 30000,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const corpus = {
  baseline: BASELINE,
  exporter_sha256: digest(await fs.readFile(new URL(import.meta.url))),
  observer_source_sha256: digest(
    await fs.readFile("tests/migration/runtime/logging-oracle.rs"),
  ),
  observer_binary_sha256: digest(await fs.readFile(executable)),
  output: JSON.parse(result.stdout),
};
await fs.writeFile(
  "tests/migration/runtime/log-compact-vectors.json",
  JSON.stringify(corpus, null, 2) + "\n",
);
console.log(corpus.output);
