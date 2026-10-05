/** Cleanup failures must invalidate a receipt without abandoning the remaining resources. */
import assert from "node:assert/strict";
import test from "node:test";
import { cleanup } from "./cleanup.mjs";

test("cleanup attempts all resources after removal and daemon failures", () => {
  const calls = [];
  const results = cleanup(
    (executable, args) => {
      calls.push(args);
      if (args.includes("first")) throw new Error("daemon unavailable");
      return { status: 1, stdout: "", stderr: "daemon unavailable" };
    },
    new Set(["first", "second"]),
    "network",
  );
  assert.equal(results.length, 3);
  assert(results.every((item) => !item.success));
  assert(calls.some((args) => args.includes("second")));
  assert(calls.some((args) => args.includes("network")));
});

test("only a successful empty inventory establishes an already absent resource", () => {
  for (const [status, stdout, expected] of [
    [0, "", true],
    [0, "container-id", false],
    [1, "", false],
  ]) {
    const [result] = cleanup(
      (executable, args) =>
        args[0] === "rm"
          ? { status: 1, stdout: "", stderr: "remove failed" }
          : { status, stdout, stderr: "" },
      new Set(["fixture"]),
    );
    assert.equal(result.success, expected);
  }
});
