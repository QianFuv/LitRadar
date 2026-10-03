/** Freeze serde numeric representations from raw JSON tokens, retaining exact float bits. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const inputs = new Set([
  "0",
  "-0",
  "0.0",
  "-0e0",
  "0.49999999999999997",
  "18446744073709551616.0",
  "9223372036854775807",
  "9223372036854775808",
  "-9223372036854775808",
  "-9223372036854775809",
  "18446744073709551615",
  "18446744073709551616",
  "1e2147483648",
  "1e-2147483648",
  "-0e2147483648",
  "1e0000000000000000000000000000000000001",
  "1.7976931348623157e308",
  "1.7976931348623158e308",
  "1.7976931348623159e308",
]);
for (const significant of [
  "0",
  "1",
  "2",
  "1234567890123456789",
  "18446744073709551615",
  "18446744073709551616",
  "123456789012345678901234567890",
  "0.49999999999999997",
  "0.00000000000000000001234567890123456789",
  "1.000000000000000000000000000000000000001",
])
  for (const exponent of [
    -2147483648, -9999, -400, -324, -323, -309, -308, -100, -23, -1, 0, 1, 23,
    100, 289, 308, 309, 400, 2147483647,
  ])
    for (const sign of ["", "-"])
      inputs.add(`${sign}${significant}e${exponent}`);
let state = 0x713219aa;
/** Generate repeatable non-secret decimal stress inputs without floating-point conversion. */
function next() {
  state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
  return state;
}
for (let index = 0; index < 1200; index++) {
  let significant = String((next() % 9) + 1);
  const width = next() % 60;
  for (let position = 0; position < width; position++)
    significant += String(next() % 10);
  const point = (next() % significant.length) + 1;
  if (point < significant.length)
    significant = significant.slice(0, point) + "." + significant.slice(point);
  inputs.add(
    (next() % 2 ? "-" : "") +
      significant +
      "e" +
      String(Number(next() % 700) - 350),
  );
}
const requests = [...inputs].map((input) => ({ kind: "number", input }));
const result = spawnSync(
  "output/migration/execution/sources-transport-oracle.exe",
  [],
  {
    input: requests.map((item) => JSON.stringify(item)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, requests.length);
await fs.writeFile(
  "tests/migration/sources/number-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-transport-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-numbers.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust numeric observations`);
