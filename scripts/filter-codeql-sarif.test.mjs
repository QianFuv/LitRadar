/**
 * Regression tests for exact reviewed CodeQL SARIF filtering.
 */

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  filterReviewedFindings,
  parseReviewedFindings,
} from "./filter-codeql-sarif.mjs";

const REVIEWED_PATH = "crates/example/src/lib.rs";
const REVIEWED_FINGERPRINT = "1382ed900590088:1";
const NEW_FINGERPRINT = "fedcba9876543210:1";

/**
 * Build one minimal SARIF result.
 *
 * @param {string} fingerprint - Primary-location fingerprint.
 * @returns {object} Minimal SARIF result.
 */
function sarifResult(fingerprint) {
  return {
    ruleId: "rust/hard-coded-cryptographic-value",
    locations: [
      {
        physicalLocation: {
          artifactLocation: { uri: REVIEWED_PATH },
          region: { startLine: 10 },
        },
      },
    ],
    partialFingerprints: { primaryLocationLineHash: fingerprint },
  };
}

/**
 * Build one reviewed-finding allowlist.
 *
 * @param {object[]} findings - Exact findings to allow.
 * @param {string} expiresOn - Allowlist expiry date.
 * @returns {object} Parsed-JSON-compatible allowlist.
 */
function reviewedAllowlist(findings, expiresOn = "2999-12-31") {
  return {
    schemaVersion: 1,
    ruleId: "rust/hard-coded-cryptographic-value",
    owner: "security-maintainers",
    rationale: "Deterministic test-only credential fixture",
    expiresOn,
    findings,
  };
}

/**
 * Create temporary SARIF and allowlist files.
 *
 * @param {object[]} results - SARIF results to write.
 * @param {object} allowlist - Allowlist to write.
 * @returns {Promise<{ root: string, sarifPath: string, allowlistPath: string }>} Fixture paths.
 */
async function createFixture(results, allowlist) {
  const root = await mkdtemp(path.join(os.tmpdir(), "litradar-codeql-filter-"));
  const sarifPath = path.join(root, "results.sarif");
  const allowlistPath = path.join(root, "allowlist.json");
  await writeFile(
    sarifPath,
    JSON.stringify({ version: "2.1.0", runs: [{ results }] }),
    "utf8",
  );
  await writeFile(allowlistPath, JSON.stringify(allowlist), "utf8");
  return { root, sarifPath, allowlistPath };
}

/**
 * Remove one generated test fixture root.
 *
 * @param {string} root - Generated fixture root.
 * @returns {Promise<void>} Completes after the fixture is removed.
 */
async function removeFixture(root) {
  const expectedParent = path.resolve(os.tmpdir());
  const resolvedRoot = path.resolve(root);
  if (
    path.dirname(resolvedRoot) !== expectedParent ||
    !path.basename(resolvedRoot).startsWith("litradar-codeql-filter-")
  ) {
    throw new Error(`refusing to remove an unexpected fixture root: ${root}`);
  }
  await rm(resolvedRoot, { recursive: true, force: false });
}

/**
 * Prove that only an exact reviewed fingerprint is removed.
 *
 * @returns {Promise<void>} Completes after assertions pass.
 */
async function filtersOnlyExactReviewedFingerprint() {
  const reviewedFinding = {
    path: REVIEWED_PATH,
    fingerprint: REVIEWED_FINGERPRINT,
  };
  const fixture = await createFixture(
    [sarifResult(REVIEWED_FINGERPRINT), sarifResult(NEW_FINGERPRINT)],
    reviewedAllowlist([reviewedFinding]),
  );
  try {
    const summary = await filterReviewedFindings(fixture);
    assert.deepEqual(summary, {
      reviewedCount: 1,
      remainingCount: 1,
      sarifFileCount: 1,
      expiresOn: "2999-12-31",
    });
    const filtered = JSON.parse(await readFile(fixture.sarifPath, "utf8"));
    assert.deepEqual(filtered.runs[0].results, [sarifResult(NEW_FINGERPRINT)]);
  } finally {
    await removeFixture(fixture.root);
  }
}

/**
 * Prove that a stale reviewed finding cannot remain silently.
 *
 * @returns {Promise<void>} Completes after assertions pass.
 */
async function rejectsUnusedReviewedFinding() {
  const fixture = await createFixture(
    [sarifResult(NEW_FINGERPRINT)],
    reviewedAllowlist([
      { path: REVIEWED_PATH, fingerprint: REVIEWED_FINGERPRINT },
    ]),
  );
  try {
    await assert.rejects(
      filterReviewedFindings(fixture),
      /unused reviewed findings/u,
    );
  } finally {
    await removeFixture(fixture.root);
  }
}

/**
 * Prove that expired review authority is rejected.
 */
function rejectsExpiredAllowlist() {
  const allowlist = reviewedAllowlist(
    [{ path: REVIEWED_PATH, fingerprint: REVIEWED_FINGERPRINT }],
    "2026-01-01",
  );
  assert.throws(
    () =>
      parseReviewedFindings(
        JSON.stringify(allowlist),
        new Date("2026-01-02T00:00:00Z"),
      ),
    /expired/u,
  );
}

/**
 * Prove that duplicate fingerprints cannot broaden one exception.
 */
function rejectsDuplicateReviewedFinding() {
  const finding = { path: REVIEWED_PATH, fingerprint: REVIEWED_FINGERPRINT };
  const allowlist = reviewedAllowlist([finding, finding]);
  assert.throws(
    () => parseReviewedFindings(JSON.stringify(allowlist)),
    /duplicate reviewed finding/u,
  );
}

test(
  "filters only an exact reviewed fingerprint",
  filtersOnlyExactReviewedFingerprint,
);
test("rejects an unused reviewed finding", rejectsUnusedReviewedFinding);
test("rejects an expired allowlist", rejectsExpiredAllowlist);
test("rejects duplicate reviewed findings", rejectsDuplicateReviewedFinding);
