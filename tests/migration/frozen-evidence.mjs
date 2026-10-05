/** Validate retained oracle bytes independently of retired compiler inputs. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest } from "./oracle.mjs";

/** Read the committed identity registry rather than trusting a neighboring build record. */
async function registry() {
  const result = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/data/migration/frozen-evidence.json"),
      "utf8",
    ),
  );
  assert.equal(result.format, 1);
  assert.equal(result.baseline, BASELINE);
  return result;
}

/** Require every retained corpus, observer and compilation record to match its frozen identity. */
export async function verifyFrozenEvidence() {
  const manifest = await registry();
  assert(manifest.files.length > 0);
  const names = new Set();
  for (const entry of manifest.files) {
    assert(!names.has(entry.path), `Duplicate frozen path: ${entry.path}`);
    names.add(entry.path);
    assert(
      !path.isAbsolute(entry.path) && !entry.path.split("/").includes(".."),
    );
    const filename = path.join(WORKSPACE_ROOT, entry.path);
    const metadata = await fs.lstat(filename);
    assert(
      metadata.isFile() && !metadata.isSymbolicLink(),
      `Invalid frozen file: ${entry.path}`,
    );
    assert.equal(
      digest(await fs.readFile(filename)),
      entry.sha256,
      `Changed frozen evidence: ${entry.path}`,
    );
  }
  return {
    baseline: BASELINE,
    files: manifest.files.length,
    status: "Verified retained bytes; historical compilation was not repeated",
  };
}

/** Check declared historical input identity without loading stale Cargo artifacts or retired sources. */
export async function verifyHistoricalInput(input) {
  const manifest = await registry();
  assert(
    manifest.historicalInputs.some(
      (entry) => entry.path === input.path && entry.sha256 === input.sha256,
    ),
    `Unrecognized historical input: ${input.path}`,
  );
}
