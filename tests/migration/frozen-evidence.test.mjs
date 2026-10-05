/** Historical provenance must reject changed identities even when old source files are absent. */
import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs/promises";
import { verifyHistoricalInput } from "./frozen-evidence.mjs";

test("retired source identity remains exact and unknown inputs fail closed", async () => {
  const manifest = JSON.parse(
    await fs.readFile("tests/data/migration/frozen-evidence.json", "utf8"),
  );
  const original = manifest.historicalInputs.find(
    (entry) => entry.path === "Cargo.lock",
  );
  assert(original);
  await verifyHistoricalInput(original);
  await assert.rejects(
    verifyHistoricalInput({ ...original, sha256: "0".repeat(64) }),
  );
  await assert.rejects(
    verifyHistoricalInput({ ...original, path: "unrecorded-input" }),
  );
});
