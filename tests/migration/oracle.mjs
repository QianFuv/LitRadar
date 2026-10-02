/**
 * Read immutable, explicitly prepared Rust oracle artifacts for migration checks.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const WORKSPACE_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
export const BASELINE = "6bb1220059c82c19a53a1418fc376fc842fea834";

/**
 * Hash exact bytes without newline or text normalization.
 * @param {Uint8Array} bytes - Artifact bytes.
 * @returns {string} SHA256 digest.
 */
export function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/**
 * Resolve a platform-specific archive without accepting caller-controlled paths.
 * @param {string} baseline - Full approved baseline commit.
 * @returns {string} Absolute artifact directory.
 */
export function oracleDirectory(baseline) {
  assert.equal(baseline, BASELINE, "Only the approved baseline is supported");
  return path.join(
    WORKSPACE_ROOT,
    "output/migration/oracle",
    baseline,
    `${process.platform}-${process.arch}`,
  );
}

/**
 * Verify every archived artifact before using the baseline executable or expected values.
 * @param {string} baseline - Approved baseline commit.
 * @returns {Promise<{directory: string, manifest: object}>} Verified archive.
 */
export async function loadOracle(baseline) {
  const directory = oracleDirectory(baseline);
  const identity = JSON.parse(
    await fs.readFile(
      path.join(
        WORKSPACE_ROOT,
        "tests/data/migration",
        `oracle-${process.platform}-${process.arch}.json`,
      ),
      "utf8",
    ),
  );
  return verifyOracle(directory, identity, baseline);
}

/**
 * Validate an archive against an independently supplied frozen identity.
 * @param {string} directory - Archive directory, never inferred from manifest content.
 * @param {object} identity - Trusted identity from the committed fixture registry.
 * @param {string} baseline - Expected baseline commit.
 * @returns {Promise<{directory: string, manifest: object}>} Verified archive.
 */
export async function verifyOracle(directory, identity, baseline) {
  assert.equal(baseline, BASELINE);
  assert.equal(identity.baseline, baseline);
  assert.equal(identity.platform, process.platform);
  assert.equal(identity.architecture, process.arch);
  assert(
    (await fs.lstat(directory)).isDirectory() &&
      !(await fs.lstat(directory)).isSymbolicLink(),
    "Linked oracle root",
  );
  await assertContainedRegularFile(directory, "manifest.json");
  const manifestBytes = await fs.readFile(
    path.join(directory, "manifest.json"),
  );
  assert.equal(
    digest(manifestBytes),
    identity.manifestSha256,
    "Frozen manifest identity changed",
  );
  const manifest = JSON.parse(manifestBytes);
  assert.equal(manifest.format, 1);
  assert.equal(manifest.syntheticOnly, true);
  assert.equal(manifest.baseline, baseline);
  assert.equal(manifest.platform, process.platform);
  assert.equal(manifest.architecture, process.arch);
  assert(manifest.files.length > 0, "Empty oracle manifest");
  const names = new Set();
  for (const artifact of manifest.files) {
    assert(!names.has(artifact.path), "Duplicate oracle artifact");
    names.add(artifact.path);
    await assertContainedRegularFile(directory, artifact.path);
    const filename = path.join(directory, artifact.path);
    const bytes = await fs.readFile(filename);
    assert.equal(
      bytes.length,
      artifact.bytes,
      `Oracle size changed: ${artifact.path}`,
    );
    assert.equal(
      digest(bytes),
      artifact.sha256,
      `Oracle bytes changed: ${artifact.path}`,
    );
  }
  for (const role of ["application", "exporter", "fullStackFixture"]) {
    assert(
      typeof manifest.binaries?.[role] === "string" &&
        names.has(manifest.binaries[role]),
      `Unverified binary role: ${role}`,
    );
  }
  const actual = [];
  /**
   * Enumerate the complete archive so an extra native dependency cannot evade hashing.
   * @param {string} relative - Owned archive-relative directory.
   * @returns {Promise<void>}
   */
  async function enumerate(relative) {
    for (const entry of await fs.readdir(path.join(directory, relative), {
      withFileTypes: true,
    })) {
      assert(!entry.isSymbolicLink(), "Linked oracle entry");
      const filename = path.posix.join(relative, entry.name);
      if (entry.isDirectory()) await enumerate(filename);
      else {
        assert(entry.isFile());
        actual.push(filename);
      }
    }
  }
  await enumerate("");
  assert.deepEqual(
    actual.sort(),
    [...names, "manifest.json"].sort(),
    "Unexpected oracle file set",
  );
  for (const required of [
    "fixtures/openapi.json",
    "fixtures/crypto.json",
    "fixtures/auth-v19-input.sqlite",
    "fixtures/auth-schema.json",
    "fixtures/content-schema.json",
    "fixtures/control-schema.json",
  ]) {
    assert(names.has(required), `Missing required fixture: ${required}`);
  }
  return { directory, manifest };
}

/**
 * Reject path aliases and links in every component before reading an oracle artifact.
 * @param {string} directory - Archive root.
 * @param {string} relative - Canonical manifest-relative path.
 * @returns {Promise<void>}
 */
async function assertContainedRegularFile(directory, relative) {
  assert(
    typeof relative === "string" &&
      relative &&
      !relative.includes("\\") &&
      !path.isAbsolute(relative) &&
      !relative.includes(":") &&
      relative
        .split("/")
        .every((part) => part && part !== "." && part !== ".."),
    "Oracle path escape",
  );
  let current = directory;
  for (const component of relative.split("/")) {
    current = path.join(current, component);
    assert(!(await fs.lstat(current)).isSymbolicLink(), "Linked oracle path");
  }
  assert(
    (await fs.lstat(current)).isFile(),
    "Oracle artifact must be a regular file",
  );
  const canonicalRoot = await fs.realpath(directory);
  const canonicalFile = await fs.realpath(current);
  assert(
    canonicalFile.startsWith(`${canonicalRoot}${path.sep}`),
    "Oracle escaped canonical root",
  );
}
