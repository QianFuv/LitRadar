/** Test that corrupted or aliased oracle inputs cannot become expected answers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { BASELINE, digest, verifyOracle } from "./oracle.mjs";

/**
 * Build a tiny owned archive for integrity checks without touching the real oracle.
 * @param {import('node:test').TestContext} context - Test cleanup owner.
 * @returns {Promise<object>} Synthetic archive and publishing helper.
 */
async function fixture(context) {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-oracle-validation-"),
  );
  context.after(async () => {
    assert.equal(path.dirname(directory), path.resolve(os.tmpdir()));
    await fs.rm(directory, { recursive: true });
  });
  const names = [
    "application",
    "exporter",
    "seeder",
    "fixtures/openapi.json",
    "fixtures/crypto.json",
    "fixtures/auth-v19-input.sqlite",
    "fixtures/auth-schema.json",
    "fixtures/content-schema.json",
    "fixtures/control-schema.json",
  ];
  await fs.mkdir(path.join(directory, "fixtures"));
  const bytes = Buffer.from("synthetic test bytes");
  for (const name of names)
    await fs.writeFile(path.join(directory, name), bytes);
  const manifest = {
    format: 1,
    baseline: BASELINE,
    platform: process.platform,
    architecture: process.arch,
    syntheticOnly: true,
    binaries: {
      application: "application",
      exporter: "exporter",
      fullStackFixture: "seeder",
    },
    files: names.map((name) => ({
      path: name,
      bytes: bytes.length,
      sha256: digest(bytes),
    })),
  };
  /**
   * Publish an explicitly trusted test identity to exercise inner manifest validation.
   * @returns {Promise<object>} Expected identity.
   */
  async function publish() {
    const encoded = Buffer.from(JSON.stringify(manifest));
    await fs.writeFile(path.join(directory, "manifest.json"), encoded);
    return {
      baseline: BASELINE,
      platform: process.platform,
      architecture: process.arch,
      manifestSha256: digest(encoded),
    };
  }
  return { directory, manifest, publish };
}

test("frozen manifest rejects same-size fixture corruption and changed manifest bytes", async (context) => {
  const { directory, publish } = await fixture(context);
  const identity = await publish();
  await verifyOracle(directory, identity, BASELINE);
  await fs.writeFile(
    path.join(directory, "application"),
    "Synthetic test bytes",
  );
  await assert.rejects(
    verifyOracle(directory, identity, BASELINE),
    /Oracle bytes changed/,
  );
  await fs.appendFile(path.join(directory, "manifest.json"), "\n");
  await assert.rejects(
    verifyOracle(directory, identity, BASELINE),
    /Frozen manifest identity changed/,
  );
});

test("manifest rejects path escapes, duplicate entries and unverified executable roles", async (context) => {
  const { directory, manifest, publish } = await fixture(context);
  const original = manifest.files[0].path;
  for (const unsafe of [
    "../application",
    "/application",
    "C:/application",
    "fixtures/../application",
    "fixtures\\openapi.json",
  ]) {
    manifest.files[0].path = unsafe;
    await assert.rejects(
      verifyOracle(directory, await publish(), BASELINE),
      /Oracle path escape/,
    );
  }
  manifest.files[0].path = original;
  manifest.files.push(manifest.files[0]);
  await assert.rejects(
    verifyOracle(directory, await publish(), BASELINE),
    /Duplicate oracle artifact/,
  );
  manifest.files.pop();
  manifest.binaries.application = "not-verified";
  await assert.rejects(
    verifyOracle(directory, await publish(), BASELINE),
    /Unverified binary role/,
  );
});

test("missing required fixtures and wrong target identity fail closed", async (context) => {
  const { directory, manifest, publish } = await fixture(context);
  manifest.files = manifest.files.filter(
    (entry) => entry.path !== "fixtures/crypto.json",
  );
  await fs.unlink(path.join(directory, "fixtures/crypto.json"));
  await assert.rejects(
    verifyOracle(directory, await publish(), BASELINE),
    /Missing required fixture/,
  );
  const identity = await publish();
  identity.architecture = "unsupported";
  await assert.rejects(verifyOracle(directory, identity, BASELINE));
});

test("unmanifested native files and directory junctions are rejected", async (context) => {
  const { directory, publish } = await fixture(context);
  const identity = await publish();
  await fs.writeFile(path.join(directory, "extra-native.dll"), "injected");
  await assert.rejects(
    verifyOracle(directory, identity, BASELINE),
    /Unexpected oracle file set/,
  );
  await fs.unlink(path.join(directory, "extra-native.dll"));
  await fs.symlink(
    path.join(directory, "fixtures"),
    path.join(directory, "linked-fixtures"),
    "junction",
  );
  await assert.rejects(
    verifyOracle(directory, identity, BASELINE),
    /Linked oracle entry/,
  );
  await fs.unlink(path.join(directory, "linked-fixtures"));
});
