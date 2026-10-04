/** Freeze typed job, timezone, cron and process argument observations from original Rust. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
cases.push({ input: { op: "zones" } });
/** Register one independent pure observation. */
function add(input) {
  cases.push({ input });
}
for (const state of [
  "idle",
  "pending",
  "claimed",
  "running",
  "success",
  "failed",
  "timed_out",
  "error",
  "unknown",
  "cancelled",
]) {
  add({ op: "state", raw: JSON.stringify(state) });
  add({ op: "state", raw: JSON.stringify({ [state]: null }) });
}
for (const raw of [
  '"timeout"',
  '""',
  '"legacy"',
  "null",
  "{}",
  "[]",
  '{"success":null,"failed":null}',
  '{"success":null,"success":null}',
  '{"success":true}',
])
  add({ op: "state", raw });
for (const raw of [
  "{}",
  "null",
  "[]",
  '{"kind":"index"}',
  '{"kind":"notify"}',
  '{"kind":"push"}',
  '{"kind":"Index"}',
  '{"kind":"shell"}',
  '{"kind":"index","notify":null}',
  '{"kind":"index","notify":1}',
  '{"kind":"index","notify":true,"push":true}',
  '{"kind":"index","metadata_file":null}',
  '{"kind":"index","metadata_file":"a.csv","metadata_file":null}',
  '{"kind":"index","kind":"index"}',
  '{"kind":"index","database":"a.sqlite"}',
  '{"kind":"notify","max_candidates":null}',
  '{"kind":"notify","max_candidates":18446744073709551615}',
  '{"kind":"notify","max_candidates":18446744073709551616}',
  '{"kind":"notify","max_candidates":1.0}',
  '{"kind":"notify","max_candidates":1e0}',
  '{"kind":"notify","max_candidates":-0}',
  '{"kind":"notify","database":"\\ud800"}',
  '{"kind":"notify","unknown":1e999}',
  '{"kind":"index","Notify":true}',
])
  add({ op: "job", raw });
for (const kind of ["index", "notify", "push"]) {
  for (const filename of [
    "a.csv",
    "a.sqlite",
    "../a.csv",
    ".hidden.csv",
    "a..b.csv",
    "a/b.csv",
    "a\\b.csv",
    "a.csv ",
    "a.CSV",
    "a:b.csv",
    "-a.csv",
    "a_.csv",
    "résumé.csv",
    "a\u2028.csv",
    "",
    "a".repeat(124) + ".csv",
    "a".repeat(125) + ".csv",
    "a.0.csv",
    "_a.sqlite",
  ]) {
    add({
      op: "job",
      raw: JSON.stringify({
        kind,
        [kind === "index" ? "metadata_file" : "database"]: filename,
      }),
    });
  }
  for (const max_candidates of [0, 1, 1000, 1001, -1])
    if (kind !== "index")
      add({ op: "job", raw: JSON.stringify({ kind, max_candidates }) });
}
for (const timezone of [
  "UTC",
  "Etc/UTC",
  "GMT",
  "GMT0",
  "EST",
  "CET",
  "US/Eastern",
  "Asia/Shanghai",
  "America/New_York",
  "Europe/Paris",
  "Australia/Lord_Howe",
  "Asia/Kathmandu",
  "Etc/GMT+8",
  "Local",
  "localtime",
  "posix/UTC",
  "right/UTC",
  "utc",
  "",
  "../UTC",
  "Factory",
]) {
  for (const timeout of [0, 1, 86400, 86401])
    add({ op: "timing", timezone, timeout });
}
const expressions = [
  "* * * * *",
  "0 0 * * *",
  "*/7 1-23/3 1,15 * MON-FRI",
  "0 0 1 * MON",
  "0 0 */2 * MON",
  "0 0 1-31/2 * *",
  "0 0 * * 7",
  "0 0 * * 0",
  "0 0 * * 0-7/2",
  "0 0 * * 1-7/2",
  "0 0 * * 6-7/2",
  "0 0 * * 7/2",
  "0 0 * * sun",
  "0 0 * JAN mon",
  "5/3 * * * *",
  "1,2,3 * * * *",
  "",
  "* * * *",
  "* * * * * *",
  "60 * * * *",
  "0 24 * * *",
  "0 0 0 * *",
  "0 0 * 13 *",
  "0 0 * * 8",
  "*/0 * * * *",
  "*/-1 * * * *",
  "*/+2 * * * *",
  "1-0 * * * *",
  ", * * * *",
  "0 0 * * MONDAY",
  "0 0 * * 7-0",
  "0 0 * * 0/9223372036854775808",
];
for (const expression of expressions) {
  add({
    op: "cron",
    expression,
    timezone: "UTC",
    from: 1704067200,
    to: 1704672000,
  });
}
for (const interval of [
  {
    timezone: "America/New_York",
    from: Date.parse("2025-03-09T00:00:00Z") / 1000,
    to: Date.parse("2025-03-10T00:00:00Z") / 1000,
  },
  {
    timezone: "America/New_York",
    from: Date.parse("2025-11-02T00:00:00Z") / 1000,
    to: Date.parse("2025-11-03T00:00:00Z") / 1000,
  },
  {
    timezone: "Australia/Lord_Howe",
    from: Date.parse("2025-04-05T00:00:00Z") / 1000,
    to: Date.parse("2025-04-07T00:00:00Z") / 1000,
  },
  { timezone: "UTC", from: -120.001, to: 0 },
  { timezone: "Asia/Shanghai", from: 59.999, to: 120.001 },
  { timezone: "Local", from: 0, to: 60 },
  { timezone: "UTC", from: 60, to: 0 },
])
  for (const expression of ["30 1 * * *", "30 2 * * *", "*/15 * * * *"])
    add({ op: "cron", expression, ...interval });
for (const job of [
  { kind: "index" },
  { kind: "index", metadata_file: "a.csv", notify: true, push: true },
  { kind: "notify", database: "a.sqlite", max_candidates: 8 },
  { kind: "push" },
  { kind: "push", database: "bad/filename.sqlite" },
])
  add({
    op: "commands",
    job,
    root: "D:/Project root",
    auth: "data with spaces/auth.sqlite",
    executable: "bin/litradar app",
    key: "secret directory/key",
  });
for (const terminal of [
  "success",
  "cancelled",
  "timeout",
  "heartbeat",
  "exit",
  "supervision",
]) {
  for (const bytes of [
    [],
    [65, 66, 10],
    [0xff, 0xfe, 0xe4, 0xb8],
    [0xe4, 0xb8, 0xad, 0xf0, 0x9f, 0x8c, 0x90],
    Array(5000).fill(65),
  ]) {
    add({ op: "summary", terminal, code: 3, bytes });
  }
  if (terminal === "exit")
    add({ op: "summary", terminal, code: null, bytes: [] });
}
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/scheduler-oracle/worker-build.json",
    "utf8",
  ),
);
const result = spawnSync(build.binary, [], {
  input: cases.map((entry) => JSON.stringify(entry.input)).join("\n") + "\n",
  encoding: "utf8",
  timeout: 120000,
  maxBuffer: 64 * 1024 * 1024,
  windowsHide: true,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const outputs = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(outputs.length, cases.length);
for (let index = 0; index < cases.length; index++)
  cases[index].expected = outputs[index];
assert.equal(
  await fs.readFile("internal/platform/cron/timezones.txt", "utf8"),
  outputs[0].join("\n") + "\n",
);
await fs.writeFile(
  "tests/migration/scheduler/worker-vectors.json",
  JSON.stringify(
    {
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/scheduler/export-worker.mjs"),
      ),
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log(JSON.stringify({ observations: cases.length }));
