/** Check formatting of authored Go application and test sources. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
const files = [];
/** Collect authored Go sources without vendored dependency rewrites. */
async function collect(directory) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) await collect(filename);
    else if (entry.isFile() && entry.name.endsWith(".go")) files.push(filename);
  }
}
for (const directory of ["cmd", "internal"]) await collect(directory);
const result = spawnSync("gofmt", ["-l", ...files.sort()], {
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
console.log(`Go formatting passed: ${files.length} files`);
