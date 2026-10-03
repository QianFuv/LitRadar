/** Negative controls for reproducible dependency patch application. */
import assert from "node:assert/strict";
import test from "node:test";
import { applyUnified } from "./dependency.mjs";

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
