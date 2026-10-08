/** Negative controls for reproducible dependency patch application. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  applyUnified,
  verifyDependency,
} from "../scripts/verify-go-dependency.mjs";

test("unified patch preserves unaffected lines and validates exact source context", () => {
  const patch = "--- a/file\n+++ b/file\n@@ -2,1 +2,1 @@\n-b\n+B\n";
  assert.equal(applyUnified("a\nb\nc\n", patch), "a\nB\nc\n");
  assert.throws(
    () => applyUnified("a\nchanged\nc\n", patch),
    /context mismatch/,
  );
  assert.throws(
    () => applyUnified("a\nb\nc\n", patch.replace("-2,1", "-2,2")),
    /removed count/,
  );
  assert.throws(
    () => applyUnified("a\nb\nc\n", patch.replace("+2,1", "+2,2")),
    /added count/,
  );
});

/** Verify insertion, deletion and multi-hunk byte preservation. */
function appliesIndependentHunks() {
  assert.equal(
    applyUnified(
      "a\nb\nc\nd\n",
      "@@ -2,1 +2,1 @@\n-b\n+B\n@@ -4,1 +4,2 @@\n d\n+D\n",
    ),
    "a\nB\nc\nd\nD\n",
  );
  assert.equal(
    applyUnified("", "@@ -0,0 +99,1 @@ accepted suffix\n+added\n"),
    "added\n",
  );
  assert.equal(applyUnified("a\n", "@@ -1,1 +0,0 @@\n-a\n"), "");
  assert.equal(
    applyUnified("a\nb\n", "@@ -2,0 +2,1 @@\n+inserted\n"),
    "a\ninserted\nb\n",
  );
}

/** Reject malformed contexts, overlapping hunks and unsupported newline syntax. */
function rejectsMalformedHunks() {
  assert.throws(
    () => applyUnified("a\n", "@@ -2,1 +2,1 @@\n-a\n+A\n"),
    /context mismatch/,
  );
  assert.throws(
    () => applyUnified("a\n", "@@ -3,0 +1,0 @@\n"),
    /Overlapping patch/,
  );
  assert.throws(
    () => applyUnified("a\n", "@@ -1,1 +1,1 @@\n a\n@@ -1,1 +1,1 @@\n a\n"),
    /Overlapping patch/,
  );
  assert.throws(
    () => applyUnified("a\n", "@@ malformed\n"),
    /Invalid patch hunk/,
  );
  assert.throws(
    () => applyUnified("a\n", "@@ -1,1 +1,1 @@\n?bad\n"),
    /Unsupported patch syntax/,
  );
  assert.throws(
    () => applyUnified("a\n", "@@ -1,1 +1,1 @@\n a\n\n+bad\n"),
    /Empty patch line/,
  );
  assert.throws(
    () =>
      applyUnified(
        "a\n",
        "@@ -1,1 +1,1 @@\n a\n\\ No newline at end of file\n",
      ),
    /Unsupported patch syntax/,
  );
  assert.throws(
    () => applyUnified("a\n", "header without hunks\n"),
    /Empty patch/,
  );
  assert.throws(
    () => applyUnified("a\r\n", ""),
    /Unexpected upstream line endings/,
  );
  assert.throws(() => applyUnified("a", ""), /Missing original final newline/);
}

/** Snapshot exact dependency-tree and patch-record bytes, including hidden files. */
async function snapshotTree(directory, relative = "") {
  const entries = await fs.readdir(path.join(directory, relative), {
    withFileTypes: true,
  });
  entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
  const result = [];
  for (const entry of entries) {
    const filename = path.join(relative, entry.name);
    if (entry.isDirectory()) {
      result.push(...(await snapshotTree(directory, filename)));
    } else {
      const bytes = await fs.readFile(path.join(directory, filename));
      result.push([filename, createHash("sha256").update(bytes).digest("hex")]);
    }
  }
  return result;
}

/** Verify both replacements without regenerating dependency or patch bytes. */
async function verifiesWithoutRegeneration(context) {
  const protectedDirectory = fileURLToPath(
    new URL("../third_party/", import.meta.url),
  );
  const writeFile = fs.writeFile;
  /** Reject dependency and patch writes while allowing integrity reports and archive caching. */
  async function writeVerificationOutput(filename, ...writeArguments) {
    const relative = path.relative(protectedDirectory, path.resolve(filename));
    const isProtected =
      relative === "" ||
      (relative !== ".." &&
        !relative.startsWith(`..${path.sep}`) &&
        !path.isAbsolute(relative));
    assert(
      !isProtected,
      "Verification must not regenerate dependency or patch records",
    );
    return writeFile(filename, ...writeArguments);
  }
  context.mock.method(fs, "writeFile", writeVerificationOutput);
  await assert.rejects(
    fs.writeFile(
      path.join(protectedDirectory, "blocked.patch"),
      "must not write",
    ),
    /Verification must not regenerate/,
  );
  for (const name of ["go-sdk", "go-sqlite3"]) {
    const local = new URL(`../third_party/${name}/`, import.meta.url);
    const records = new URL(`../third_party/${name}-patches/`, import.meta.url);
    const localDirectory = fileURLToPath(local);
    const recordDirectory = fileURLToPath(records);
    const before = [
      await snapshotTree(localDirectory),
      await snapshotTree(recordDirectory),
    ];
    const report = await verifyDependency(name, false);
    assert.equal(report.dependency, name);
    assert.equal(report.result, "Passed");
    assert.match(report.patchSha256, /^[a-f0-9]{64}$/);
    assert.deepEqual(
      [await snapshotTree(localDirectory), await snapshotTree(recordDirectory)],
      before,
    );
  }
}

test(
  "applies multiple hunks and insertions or deletions without rewriting unaffected bytes",
  appliesIndependentHunks,
);
test("rejects malformed and overlapping patch syntax", rejectsMalformedHunks);
test(
  "verification leaves both dependency trees and patch records unchanged",
  verifiesWithoutRegeneration,
);
