/** Verify transferred build artifacts still match the exact tested source and image. */
import assert from "node:assert/strict";
import fs from "node:fs";
import { pathToFileURL } from "node:url";

/** Reject untested image bytes, wrong platforms and mismatched Windows provenance. */
export function verifyReleaseEvidence({
  image,
  loaded,
  linux,
  windows,
  version,
  commit,
}) {
  assert.match(commit, /^[a-f0-9]{40}$/);
  assert.equal(linux.status, "passed", "Linux image smoke must pass");
  assert.equal(windows.status, "passed", "Windows archive smoke must pass");
  assert.equal(windows.sourceCommit, commit, "Windows source differs");
  assert.equal(windows.version, version, "Windows version differs");
  assert.match(linux.imageId, /^sha256:[a-f0-9]{64}$/);
  for (const inspection of [image, loaded]) {
    assert.equal(
      inspection.Id,
      linux.imageId,
      "Image differs from tested bytes",
    );
    assert.equal(inspection.Os, "linux");
    assert.equal(inspection.Architecture, "amd64");
    assert.equal(
      inspection.Config.Labels["org.opencontainers.image.revision"],
      commit,
      "Linux source differs",
    );
    assert.equal(
      inspection.Config.Labels["org.opencontainers.image.version"],
      version,
      "Linux version differs",
    );
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const read = (filename) =>
    JSON.parse(fs.readFileSync(`release-results/${filename}`, "utf8"));
  verifyReleaseEvidence({
    image: read("image-amd64.json")[0],
    loaded: read("loaded-image.json")[0],
    linux: read("smoke-amd64.json"),
    windows: read("windows/smoke/summary.json"),
    version: process.env.RELEASE_VERSION,
    commit: process.env.GITHUB_SHA,
  });
}
