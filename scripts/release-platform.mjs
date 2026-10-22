/** Keep publication targets narrower than the platforms supported by source builds. */
import assert from "node:assert/strict";

/**
 * Reject unsupported or mismatched release targets before native build/extraction.
 * @param {string} operatingSystem - Effective image or Go target OS.
 * @param {string} architecture - Effective image or Go target architecture.
 * @param {string} expectedSystem - Platform owned by the invoking packager.
 * @returns {void} Returns only for the packager's supported amd64 target.
 */
export function assertReleasePlatform(
  operatingSystem,
  architecture,
  expectedSystem,
) {
  assert(
    ["linux", "windows"].includes(expectedSystem),
    "Unsupported release platform",
  );
  assert.equal(
    operatingSystem,
    expectedSystem,
    "Release operating system differs from packaging target",
  );
  assert.equal(architecture, "amd64", "Release architecture must be amd64");
}
