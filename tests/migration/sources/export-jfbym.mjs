/** Freeze original slider parsing, candidate ordering, prefix stripping and ciphertext. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const requests = [];
requests.push(
  {
    kind: "slider",
    input: '{"code":10000,"data":{"data":0.49999999999999997}}',
  },
  {
    kind: "slider",
    input: '{"code":10000,"data":{"data":"0.49999999999999997"}}',
  },
);
for (const input of [
  "null",
  "[]",
  "{}",
  '{"code":10000}',
  ...["10000", "10000.0", '"10000"', "0", "18446744073709551615"].map(
    (code) => `{"code":${code},"data":{"data":"261"}}`,
  ),
])
  requests.push({ kind: "slider", input });
for (const distance of [
  null,
  true,
  {},
  [],
  0,
  -0,
  261.4,
  10000,
  10000.1,
  -1,
  "261",
  " 261 ",
  "+261",
  "261.",
  ".5",
  "1e2",
  "1_00",
  "0x1p2",
  "NaN",
  "+NaN",
  "-NaN",
  "nan",
  "Inf",
  "Infinity",
  "-inf",
  "1e400",
  "1e-400",
  "10000.1",
  "距离：261",
  "",
  "  ",
])
  requests.push({
    kind: "slider",
    input: JSON.stringify({ code: 10000, data: { data: distance } }),
  });
for (const input of [
  "0",
  "-0",
  "0.49",
  "0.5",
  "1.5",
  "261.4",
  "261.5",
  "9999.5",
  "10000",
  "10000.1",
  "-1",
  "NaN",
  "inf",
  "-inf",
])
  requests.push({ kind: "points", input });
for (const input of [
  "",
  "short",
  "0123456789abcdef",
  "éééééééé",
  "验证key123456",
  "0123456789abcdefX",
])
  for (const [x, y] of [
    [261, 5],
    [0, 0],
    [-2147483648, 2147483647],
    [10000, -2],
  ])
    requests.push({ kind: "encrypt", input, x, y });
for (const input of [
  "data:image/png;base64,AAAA",
  " DATA:image/png;base64, BBBB ",
  "data:missing-comma",
  "  BBBB  ",
  "",
  "data:,",
  "data:,a,b",
  "data:,\u00a0AAAA\u00a0",
])
  requests.push({ kind: "strip", input });
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
  "tests/migration/sources/jfbym-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-transport-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-jfbym.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust JFBYM observations`);
