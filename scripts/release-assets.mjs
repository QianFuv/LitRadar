/** Validate complete release bytes and combine platform checksums before publication. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { parseVersion } from "./release-version.mjs";

/** Return the immutable platform asset names for a stable version. */
export function releaseAssetNames(inputVersion, windowsOnly = false) {
  const version = parseVersion(inputVersion);
  const windows = `litradar_${version}_windows_amd64.zip`;
  return windowsOnly
    ? [windows, `${windows}.sha256`]
    : [`litradar_${version}_linux_amd64.tar.gz`, windows, "SHA256SUMS"];
}

/** Require exact checksum membership and verify every archive before any external write. */
export function validateAssets(directory, version, windowsOnly = false) {
  const names = releaseAssetNames(version, windowsOnly);
  const checksumName = names.at(-1);
  const expectedArchives = names.slice(0, -1);
  const lines = fs
    .readFileSync(path.join(directory, checksumName), "utf8")
    .trim()
    .split(/\r?\n/);
  const checksums = new Map();
  for (const line of lines) {
    const match = /^([a-f0-9]{64})  ([a-zA-Z0-9_.-]+)$/.exec(line);
    assert(match, "Invalid release checksum line");
    assert(!checksums.has(match[2]), "Duplicate release checksum");
    checksums.set(match[2], match[1]);
  }
  assert.deepEqual(
    [...checksums.keys()].sort(),
    [...expectedArchives].sort(),
    "Release requires exactly the expected platforms",
  );
  return names.map((name) => {
    const bytes = fs.readFileSync(path.join(directory, name));
    assert(bytes.length > 0, `Empty release asset: ${name}`);
    const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
    if (checksums.has(name))
      assert.equal(
        digest,
        `sha256:${checksums.get(name)}`,
        `Checksum differs: ${name}`,
      );
    return { name, digest, size: bytes.length };
  });
}

/** Merge a verified Windows artifact into the tested Linux archive. */
function mergeAssets(version) {
  const windowsDirectory = "release-results/windows/assets";
  const assets = validateAssets(windowsDirectory, version, true);
  const directory = "release-results/assets";
  const zip = assets[0].name;
  assert(
    !fs.existsSync(path.join(directory, zip)),
    "Windows archive already merged",
  );
  fs.copyFileSync(path.join(windowsDirectory, zip), path.join(directory, zip));
  fs.appendFileSync(
    path.join(directory, "SHA256SUMS"),
    `${assets[0].digest.slice(7)}  ${zip}\n`,
  );
  validateAssets(directory, version);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  mergeAssets(parseVersion(process.argv[2]));
