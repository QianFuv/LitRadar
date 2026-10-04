/** Freeze scope, typed-field, specificity and record-update behavior from the original subscriber. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
for (const filter of [
  "",
  ",,",
  "off",
  "error",
  "info",
  "trace",
  "litradar",
  "warn,litradar=info",
  "trace,litradar=off",
  "litradar=off,litradar=debug",
  "litradar=debug,litradar=off",
  "litradar=trace,litradar_api=warn",
  "[{value}]=debug",
  "[{missing}]=trace",
  "[process]=debug",
  "off,[process]=off",
  "[process]=error",
  "[process{command=serve}]=trace",
  "[process{command=admin}]=trace",
  "[process{command=ser}]=trace",
  "[process{command=serve}]=off,info",
  "[process{command=serve}]=warn,[process]=trace",
  "litradar[process]=debug",
  "other[process]=trace",
  "[]",
  " INFO",
  "info ",
  " litradar=info",
  "litradar=info ",
  "litradar[process]=",
  "[process{value=true}]=trace",
]) {
  for (const target of [
    "litradar",
    "litradar_api::http_observability",
    "other",
  ])
    cases.push({ filter, target });
}
for (const [pattern, initial, update] of [
  ["true", { kind: "bool", value: true }, { kind: "bool", value: false }],
  ["true", { kind: "bool", value: false }, { kind: "bool", value: true }],
  ["true", { kind: "str", value: "true" }, { kind: "bool", value: true }],
  ["1", { kind: "u64", value: "1" }, { kind: "u64", value: "2" }],
  ["1", { kind: "i64", value: "1" }, null],
  ["-1", { kind: "i64", value: "-1" }, { kind: "u64", value: "1" }],
  ["-0", { kind: "u64", value: "0" }, { kind: "i64", value: "0" }],
  ["1.0", { kind: "i64", value: "1" }, { kind: "f64", value: "1" }],
  [
    "1.0",
    { kind: "f64", value: "1.0000000000000002" },
    { kind: "f64", value: "1" },
  ],
  ["NaN", { kind: "f64", value: "NaN" }, null],
  ["inf", { kind: "f64", value: "inf" }, null],
  ["1e999", { kind: "f64", value: "inf" }, null],
  ["a+b", { kind: "str", value: "ab" }, { kind: "str", value: "abb" }],
  ["a+b", { kind: "str", value: "abb" }, { kind: "str", value: "ab" }],
  ['"ab"', { kind: "debug", value: "ab" }, null],
  ["ab", { kind: "debug", value: "ab" }, { kind: "str", value: "ab" }],
  ["(?i)k", { kind: "str", value: "K" }, null],
  ["1_0", { kind: "str", value: "1_0" }, null],
  ["1_0", { kind: "f64", value: "10" }, null],
  ["0x1p2", { kind: "str", value: "0x1p2" }, null],
  ["0x1p2", { kind: "f64", value: "4" }, null],
  ["--NaN", { kind: "str", value: "--NaN" }, null],
  ["--NaN", { kind: "f64", value: "NaN" }, null],
  ["+-NaN", { kind: "f64", value: "NaN" }, null],
]) {
  for (const prefix of ["off,", "info,", "[process]=trace,"])
    cases.push({
      filter: `${prefix}[process{value=${pattern}}]=debug`,
      initial,
      update,
      target: "other",
    });
}
const binary = "output/migration/execution/logging-oracle.exe";
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/t11-logging-oracle-build.json",
    "utf8",
  ),
);
assert.equal(digest(await fs.readFile(binary)), build.binary_sha256);
const result = spawnSync(binary, [], {
  input: JSON.stringify({ filters: cases }),
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 16 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = JSON.parse(result.stdout);
assert.equal(observations.length, cases.length);
const artifact = {
  provenance: {
    kind: "original-locked-tracing-subscriber",
    source_sha256: build.source_sha256,
    binary_sha256: build.binary_sha256,
    dependencies: build.dependencies,
    exporter_sha256: digest(await fs.readFile(new URL(import.meta.url))),
  },
  cases: cases.map((input, index) => ({ ...input, ...observations[index] })),
};
await fs.writeFile(
  "tests/migration/runtime/log-filter-vectors.json",
  JSON.stringify(artifact, null, 2) + "\n",
);
console.log(
  JSON.stringify({
    filters: cases.length,
    observations: observations.reduce(
      (count, item) => count + (item.observations?.length ?? 0),
      0,
    ),
  }),
);
