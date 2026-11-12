/** Check formatting of authored Go application and test sources. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
/**
 * Authored Go roots. gofmt walks directories itself, so the command line stays short on Windows,
 * whose argument limit a per-file list exceeds.
 */
const roots = ["apps/server", "version.go"];
const result = spawnSync("gofmt", ["-l", ...roots], {
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
assert.equal(
  result.stdout.trim(),
  "",
  `Unformatted Go source:\n${result.stdout}`,
);
console.log(`Go formatting passed: ${roots.join(", ")}`);
