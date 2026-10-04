/** Freeze unchanged Rust batch state-machine histories and typed recovery decisions. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/index-oracle/identity-build.json",
    "utf8",
  ),
);
const content = JSON.parse(
  await fs.readFile("tests/migration/index/content-vectors.json", "utf8"),
);
const entry = content.observations[0].input.operations[0].catalog;
const catalog = {
  filename: "example.csv",
  name: "example",
  digest: "a".repeat(64),
  provider: "scholarly",
  entries: [entry],
};
const request = {
  catalogs: [catalog],
  selection: "all",
  mode: "incremental",
  size: 10,
  notify: true,
  dry: false,
};
const admit = { op: "admit", request };
const index = { op: "phase", phase: "indexing" };
const outcome = {
  run: "run-1",
  journals: 1,
  written: 4,
  attempts: 2,
  path: null,
};
const intent = {
  payload: [123, 125, 10],
  through: 3,
  path: "data/changes/example.json",
  run: "run-1",
  generated: "100",
};
const save = { op: "outcome", value: outcome };
const prepare = { op: "intent", value: intent };
const published = { op: "phase", phase: "manifest_published" };
const notifying = { op: "phase", phase: "notifying" };
const start = { op: "notify_prepare", attempt: "attempt-1" };
const complete = { op: "complete_catalog", value: outcome };
const prefix = [admit, index, save, prepare, published, notifying];
const requests = [];
/** Preserve each independent workflow with its final typed database snapshot. */
function scenario(name, operations, setup = "") {
  requests.push({ op: "batch", name, operations, setup });
}
scenario("admit-resume-release", [
  admit,
  { ...admit, now: 101 },
  { op: "read" },
  { op: "release" },
  { op: "release" },
  { ...admit, owner: "owner-2", now: 102 },
]);
scenario("lease-expiry-and-fence", [
  admit,
  { ...admit, owner: "other", now: 399 },
  { op: "heartbeat", owner: "other", now: 399 },
  { ...admit, owner: "other", now: 400 },
  { op: "heartbeat", now: 401 },
  { op: "release" },
  { op: "heartbeat", owner: "other", now: 700 },
  { op: "read" },
]);
scenario("abandon-and-replace", [
  admit,
  { ...admit, resume: false },
  admit,
  { ...admit, resume: false, now: 101 },
  { op: "replace", request, now: 102 },
  { op: "read" },
]);
scenario("replace-active-denied", [
  admit,
  { op: "replace", request },
  { op: "read" },
]);
scenario("lease-large-exact", [
  { ...admit, now: 9007199254740000 },
  { op: "heartbeat", now: 9007199254740000 },
]);
scenario("published-notify-blocks-abandonment", [
  admit,
  index,
  { ...save, value: { ...outcome, path: "data/changes/example.json" } },
  { ...admit, resume: false },
  { op: "read" },
]);
scenario("unpublished-intent-allows-abandonment", [
  admit,
  index,
  save,
  prepare,
  { ...admit, resume: false },
]);
scenario("notify-disabled-abandons-path", [
  { ...admit, request: { ...request, notify: false } },
  index,
  { ...save, value: { ...outcome, path: "changes.json" } },
  { ...admit, resume: false },
]);
for (const [name, patch] of Object.entries({
  selection: { selection: "explicit_file" },
  mode: { mode: "bootstrap" },
  size: { size: 11 },
  notify: { notify: false },
  dry: { dry: true },
  order: { catalogs: [{ ...catalog, filename: "other.csv" }] },
  digest: { catalogs: [{ ...catalog, digest: "A".repeat(64) }] },
  provider: { catalogs: [{ ...catalog, provider: "cnki" }] },
  all: {
    selection: "explicit_file",
    mode: "full_rescan",
    size: 12,
    notify: false,
    dry: true,
    catalogs: [{ ...catalog, digest: "b".repeat(64), provider: "cnki" }],
  },
}))
  scenario(`mismatch-${name}`, [
    admit,
    { ...admit, request: { ...request, ...patch }, owner: "other" },
  ]);
scenario("same-fingerprint-ignores-path-and-entries", [
  admit,
  {
    ...admit,
    request: {
      ...request,
      catalogs: [{ ...catalog, path: "different", entries: [] }],
    },
  },
]);
scenario("unexplained-fingerprint", [
  admit,
  { op: "sql", sql: "UPDATE index_batches SET fingerprint=printf('%064d',0)" },
  admit,
]);
for (const current of [
  "pending",
  "indexing",
  "manifest_prepared",
  "manifest_published",
  "notifying",
  "completed",
]) {
  for (const next of [
    "pending",
    "indexing",
    "manifest_prepared",
    "manifest_published",
    "notifying",
    "completed",
  ]) {
    scenario(`phase-${current}-${next}`, [
      admit,
      { op: "sql", sql: `UPDATE index_batch_catalogs SET phase='${current}'` },
      { op: "phase", phase: next },
    ]);
  }
}
scenario("outcome-before-start", [
  admit,
  save,
  complete,
  { op: "complete_batch" },
]);
scenario("outcome-monotonic-path", [
  admit,
  index,
  save,
  { ...save, value: { ...outcome, path: "changes.json" } },
  save,
  { ...save, value: { ...outcome, path: "other.json" } },
  { op: "read" },
]);
for (const [key, value] of Object.entries({
  run: "run-2",
  journals: 2,
  written: 5,
  attempts: 3,
}))
  scenario(`immutable-${key}`, [
    admit,
    index,
    save,
    { ...save, value: { ...outcome, [key]: value } },
  ]);
scenario("frozen-journal-count", [
  admit,
  index,
  { ...save, value: { ...outcome, journals: 2 } },
]);
scenario("outcome-validation", [
  admit,
  index,
  { ...save, value: { ...outcome, run: "" } },
  { ...save, value: { ...outcome, written: -1 } },
  { ...save, value: { ...outcome, path: "a/../b" } },
]);
scenario("intent-exact-replay", [
  admit,
  index,
  prepare,
  { ...prepare, now: 101 },
  published,
  { ...prepare, now: 102 },
  { ...prepare, value: { ...intent, payload: [1, 2] } },
  { op: "read" },
]);
scenario("intent-wrong-phase", [
  admit,
  prepare,
  index,
  prepare,
  { op: "sql", sql: "UPDATE index_batch_catalogs SET phase='indexing'" },
  prepare,
]);
for (const [name, patch] of Object.entries({
  empty: { payload: [] },
  cursor: { through: 0 },
  path: { path: "a/../b" },
  absolute: { path: "/a" },
  run: { run: "" },
  time: { generated: "" },
  digest: { digest: "0".repeat(64) },
  binary: { payload: [0, 255, 13] },
  dot: { path: "." },
}))
  scenario(`intent-${name}`, [
    admit,
    index,
    { ...prepare, value: { ...intent, ...patch } },
    { op: "read" },
  ]);
for (const path of [
  "1:manifest.json",
  "_:manifest.json",
  "metadata/./manifest.json",
])
  scenario(`relative-path-${path}`, [
    admit,
    index,
    { ...prepare, value: { ...intent, path } },
    { op: "read" },
  ]);
for (const exit of [2147483648, -2147483649])
  scenario(`notify-exit-range-${exit}`, [
    ...prefix,
    start,
    {
      op: "sql",
      sql: `UPDATE index_batch_catalogs SET notify_exit_code=${exit}`,
    },
    { op: "read" },
  ]);
scenario("notification-running-reuse", [
  ...prefix,
  start,
  { ...start, attempt: "ignored", now: 101 },
  { ...start, attempt: "", now: 102 },
  {
    op: "notify_record",
    attempt: "attempt-1",
    status: "running",
    exit: 1,
    now: 103,
  },
  { ...start, attempt: "ignored-again", now: 104 },
]);
for (const status of [
  "idle",
  "completed",
  "skipped",
  "failed",
  "cancelled",
  "timed_out",
  "unknown",
]) {
  scenario(`notification-${status}`, [
    ...prefix,
    start,
    {
      op: "notify_record",
      attempt: "attempt-1",
      status,
      exit: ["idle", "completed", "skipped"].includes(status) ? 0 : 1,
      now: 101,
    },
    { ...start, now: 102 },
    { ...start, attempt: "attempt-2", now: 103 },
    complete,
    { op: "read" },
  ]);
}
scenario("notification-unknown-ack", [
  ...prefix,
  start,
  { op: "notify_record", attempt: "attempt-1", status: "unknown", exit: 0 },
  { ...start, ack: true },
  { ...start, attempt: "attempt-2", ack: true, now: 102 },
  { op: "notify_record", attempt: "attempt-1", status: "completed", exit: 0 },
  { op: "notify_record", attempt: "attempt-2", status: "failed", exit: null },
  { ...start, attempt: "attempt-3", now: 103 },
  { op: "read" },
]);
scenario("notification-terminal-immutable", [
  ...prefix,
  start,
  { op: "notify_record", attempt: "attempt-1", status: "completed", exit: 0 },
  {
    op: "notify_record",
    attempt: "attempt-1",
    status: "completed",
    exit: 0,
    now: 101,
  },
  {
    op: "notify_record",
    attempt: "attempt-1",
    status: "failed",
    exit: 1,
    now: 102,
  },
  complete,
  { op: "complete_batch" },
  { op: "release" },
]);
scenario("notification-invalid-result", [
  ...prefix,
  { op: "notify_record", attempt: "attempt-1", status: "completed", exit: 0 },
  start,
  { op: "notify_record", attempt: "stale", status: "completed", exit: 0 },
  {
    op: "notify_record",
    attempt: "attempt-1",
    status: "completed",
    exit: null,
  },
  { op: "notify_record", attempt: "attempt-1", status: "failed", exit: 0 },
  complete,
]);
scenario("completion-idempotent-time", [
  admit,
  index,
  complete,
  { ...complete, now: 101 },
  { op: "complete_batch", now: 102 },
  { op: "complete_batch", now: 103 },
]);
scenario("incomplete-outcome", [
  admit,
  index,
  { op: "sql", sql: "UPDATE index_batch_catalogs SET written_article_count=1" },
  { op: "read" },
]);
scenario("incomplete-intent", [
  admit,
  index,
  { op: "sql", sql: "UPDATE index_batch_catalogs SET manifest_path='x'" },
  { op: "read" },
]);
scenario("incomplete-handoff", [
  admit,
  index,
  { op: "sql", sql: "UPDATE index_batch_catalogs SET notify_status='unknown'" },
  { op: "read" },
]);
scenario("incomplete-acknowledgement", [
  ...prefix,
  start,
  {
    op: "sql",
    sql: "UPDATE index_batch_catalogs SET notify_unknown_acknowledged_at=1",
  },
  { op: "read" },
]);
scenario("missing-catalog", [
  admit,
  { op: "phase", phase: "indexing", ordinal: 9 },
]);
for (const version of [-1, 0, 2, 3])
  scenario(`version-${version}`, [], `PRAGMA user_version=${version}`);
for (const [name, patch] of Object.entries({
  empty: { catalogs: [] },
  explicit: { selection: "explicit_file", catalogs: [catalog, catalog] },
  zero: { size: 0 },
  filename: { catalogs: [{ ...catalog, filename: "" }] },
  name: { catalogs: [{ ...catalog, name: "" }] },
  provider: { catalogs: [{ ...catalog, provider: "" }] },
  digest: { catalogs: [{ ...catalog, digest: "z".repeat(64) }] },
  duplicate: { catalogs: [catalog, catalog] },
  duplicate_name: {
    catalogs: [catalog, { ...catalog, filename: "other.csv" }],
  },
}))
  scenario(`request-${name}`, [
    { ...admit, request: { ...request, ...patch } },
  ]);
const observations = [];
for (const input of requests) {
  const result = spawnSync(build.binary, [], {
    input: JSON.stringify(input) + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  observations.push(JSON.parse(result.stdout));
}
assert.equal(
  observations.find(
    (value) => value.input.name === "notification-terminal-immutable",
  ).expected.tables.index_batches[0][1],
  "completed",
);
assert.equal(
  observations.find((value) => value.input.name === "abandon-and-replace")
    .expected.tables.index_batches.length,
  2,
);
await fs.writeFile(
  "tests/migration/index/batch-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-batch.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust batch histories`);
