/** Freeze original Rust scheduler histories without deriving expected results from Go. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
/** Register an isolated scheduler history. */
function add(name, steps) {
  cases.push({ name, steps });
}
const create = { op: "create", now: 0 };
const enqueue = { op: "enqueue", slots: [60, 120, 180] };
const claim = { op: "claim", now: 180 };
add("coalesced-success", [
  create,
  enqueue,
  claim,
  { op: "start", now: 180 },
  { op: "renew", now: 182 },
  { op: "finish", now: 183, summary: "stdout: done" },
  { op: "status", now: 190 },
]);
add("manual-does-not-consume-cron-watermark", [
  create,
  { op: "manual", now: 360 },
  { op: "start", now: 360 },
  { op: "finish", now: 361 },
  { op: "manual", now: 360 },
  { op: "finish", run: 2, now: 361 },
  { op: "enqueue", slots: [60, 120] },
  { op: "claim", now: 362 },
  { op: "manual", now: 363 },
  { op: "finish", run: 3, now: 364 },
  { op: "enqueue", slots: [360] },
  { op: "manual", now: 365 },
  { op: "claim", now: 366 },
  { op: "finish", run: 5, now: 367 },
  { op: "claim", now: 368 },
]);
for (const coalesce of [true, false]) {
  add(`expired-scheduled-claim-${coalesce}`, [
    { ...create, coalesce },
    { op: "enqueue", slots: [0] },
    { op: "claim", now: 0 },
    enqueue,
    { op: "claim", now: 120, worker: "restart" },
  ]);
  for (const started of [true, false])
    add(`expired-manual-${coalesce}-${started}`, [
      { ...create, coalesce, enabled: false },
      { op: "manual", now: 100 },
      ...(started ? [{ op: "start", now: 100 }] : []),
      { op: "claim", now: 111 },
      { op: "status", now: 111 },
      { op: "manual", now: 112 },
    ]);
  for (const status of ["claimed", "running", "success"]) {
    add(`watermark-${coalesce}-${status}`, [
      { ...create, coalesce },
      enqueue,
      claim,
      ...(status !== "claimed" ? [{ op: "start", now: 180 }] : []),
      ...(status === "success" ? [{ op: "finish", now: 181 }] : []),
      { op: "enqueue", slots: [60, 120] },
    ]);
  }
}
add("manual-automatic-busy", [
  create,
  enqueue,
  { op: "manual", now: 180 },
  { op: "manual", now: 181 },
  { op: "claim", now: 182 },
  { op: "finish", run: 2, now: 183 },
  claim,
  { op: "manual", now: 184 },
]);
add("expiration-only-after-reconcile", [
  create,
  enqueue,
  claim,
  { op: "start", now: 500 },
  { op: "renew", now: 900 },
  { op: "finish", now: 2000 },
]);
add("owner-fence-and-terminal-fence", [
  create,
  enqueue,
  claim,
  { op: "start", worker: "other" },
  { op: "renew", worker: "other" },
  { op: "finish", worker: "other" },
  { op: "start" },
  { op: "finish" },
  { op: "finish", status: "failed" },
  { op: "renew" },
]);
add("running-expiry-unknown", [
  create,
  enqueue,
  claim,
  { op: "start", now: 180 },
  { op: "claim", now: 190, worker: "restart" },
  { op: "finish", now: 191 },
  { op: "renew", now: 191 },
  { op: "status", now: 191 },
]);
add("capacity-zero-does-not-reconcile", [
  create,
  enqueue,
  claim,
  { op: "claim", now: 999, capacity: 0 },
  { op: "start", now: 1000 },
]);
add("capacity-ordered-task-wide", [
  { ...create, coalesce: false },
  { ...create, name: "second", coalesce: false },
  { op: "enqueue", slots: [300, 100, 200] },
  { op: "enqueue", task: 2, slots: [150, 50] },
  { op: "claim", now: 0, capacity: 1 },
  { op: "claim", now: 1, capacity: 2 },
  { op: "claim", now: 2, capacity: 20 },
  { op: "finish", task: 2, run: 5, now: 3 },
  { op: "claim", now: 4, capacity: 1 },
]);
add("unsorted-coalesce-uses-tail", [
  create,
  { op: "enqueue", slots: [300, 60, 120] },
  { op: "enqueue", slots: [999, 60] },
  { op: "status" },
]);
add("cursor-monotonic", [
  { op: "cursor" },
  { op: "check", now: 100 },
  { op: "check", now: 90 },
  { op: "cursor" },
  { op: "check", now: 101 },
  { op: "cursor" },
]);
add("heartbeat-preserves-start-and-prunes", [
  { op: "heartbeat", worker: "old", now: 0 },
  { op: "heartbeat", worker: "edge", now: 1 },
  { op: "heartbeat", worker: "new", now: 604801 },
  { op: "heartbeat", worker: "new", now: 604802 },
  { op: "renew", worker: "absent-run", now: 604803 },
  { op: "status", now: 604892, window: 90 },
]);
add("delete-keeps-history", [
  create,
  enqueue,
  claim,
  { op: "delete" },
  { op: "finish", now: 190 },
  { op: "status", now: 191 },
]);
add("legacy-replacement", [
  create,
  {
    op: "update",
    sql: "UPDATE scheduled_tasks SET job_spec=NULL,legacy_command='legacy shell',enabled=0",
    name: "renamed",
  },
  { op: "update", enabled: true },
  { op: "manual" },
  {
    op: "update",
    job: { kind: "notify", database: "safe.sqlite", max_candidates: 3 },
    enabled: true,
  },
  { op: "get" },
]);
for (const job of [
  { kind: "index", metadata_file: "../secret.csv" },
  { kind: "push", database: "bad.SQLITE" },
  { kind: "notify", max_candidates: 0 },
  { kind: "notify", max_candidates: 1001 },
  { kind: "index", metadata_file: "safe.csv", notify: true, push: true },
])
  add(`validation-${cases.length}`, [{ ...create, job }, { op: "list" }]);
for (const timing of [
  { timezone: "Local" },
  { timezone: "utc" },
  { timezone: "Asia/Shanghai" },
  { timeout: 0 },
  { timeout: 86401 },
])
  add(`timing-${cases.length}`, [{ ...create, ...timing }, { op: "list" }]);
for (const state of ["", "timeout", "legacy-result", "unknown"])
  add(`stored-status-${state}`, [
    create,
    { op: "get", sql: `UPDATE scheduled_tasks SET last_status='${state}'` },
  ]);
for (const mutation of [
  "timeout_seconds=-1",
  "timeout_seconds='bad'",
  "job_spec='{}'",
  "job_spec='null'",
  'job_spec=\'{"kind":"index","notify":null}\'',
  'job_spec=\'{"kind":"notify","unknown":true}\'',
  "name=x'ff'",
]) {
  add(`malformed-${cases.length}`, [
    create,
    { op: "get", sql: `UPDATE scheduled_tasks SET ${mutation}` },
  ]);
}
add("claim-commits-before-corrupt-task-read", [
  create,
  enqueue,
  { op: "claim", sql: "UPDATE scheduled_tasks SET job_spec='{}'" },
]);
add("claim-accepts-semantically-invalid-typed-job", [
  create,
  enqueue,
  {
    op: "claim",
    sql: 'UPDATE scheduled_tasks SET job_spec=\'{"kind":"push","max_candidates":0}\',timeout_seconds=0',
  },
]);
add("manual-invalid-does-not-reconcile", [
  create,
  { ...create, name: "second" },
  enqueue,
  claim,
  {
    op: "manual",
    task: 2,
    now: 999,
    sql: "UPDATE scheduled_tasks SET timeout_seconds=0 WHERE id=2",
  },
]);
add("manual-busy-commits-other-expiration", [
  create,
  { ...create, name: "second" },
  enqueue,
  claim,
  { op: "manual", task: 2, now: 181, lease: 1000 },
  { op: "manual", task: 2, now: 191 },
]);
add("missing-manual-no-reconciliation", [
  create,
  enqueue,
  claim,
  { op: "manual", task: 999, now: 999 },
]);
add("finish-rollback-on-task-trigger", [
  create,
  enqueue,
  claim,
  {
    op: "finish",
    sql: "CREATE TRIGGER reject_summary BEFORE UPDATE ON scheduled_tasks BEGIN SELECT RAISE(ABORT,'failure');END",
  },
]);
add("renew-rolls-back-worker-on-run-trigger", [
  create,
  enqueue,
  claim,
  { op: "start" },
  {
    op: "renew",
    worker: "other",
    sql: "CREATE TRIGGER reject_run BEFORE UPDATE ON scheduled_task_runs BEGIN SELECT RAISE(ABORT,'failure');END",
  },
  { op: "renew" },
]);
add("heartbeat-partial-write-is-preserved", [
  {
    op: "heartbeat",
    sql: "CREATE TRIGGER reject_service BEFORE INSERT ON service_heartbeats BEGIN SELECT RAISE(ABORT,'failure');END",
  },
]);
add("create-readback-rollback", [
  {
    ...create,
    sql: "CREATE TRIGGER erase_task AFTER INSERT ON scheduled_tasks BEGIN DELETE FROM scheduled_tasks WHERE id=NEW.id;END",
  },
]);
add("update-missing-readback-commits", [
  create,
  {
    op: "update",
    name: "changed",
    sql: "CREATE TRIGGER erase_task AFTER UPDATE ON scheduled_tasks BEGIN DELETE FROM scheduled_tasks WHERE id=NEW.id;END",
  },
]);
add("missing-cursor-contract", [
  { op: "cursor", sql: "DELETE FROM scheduler_state" },
  { op: "check" },
]);
add("nonterminal-finish-rejected", [
  create,
  enqueue,
  claim,
  { op: "finish", status: "running" },
]);

const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/scheduler-oracle/storage-build.json",
    "utf8",
  ),
);
const directory = await fs.mkdtemp(
  path.resolve("output/migration/execution/scheduler-oracle/histories-"),
);
const input =
  cases
    .map((entry, index) =>
      JSON.stringify({
        path: path.join(directory, `${index}.sqlite`),
        steps: entry.steps,
      }),
    )
    .join("\n") + "\n";
const result = spawnSync(build.binary, [], {
  input,
  encoding: "utf8",
  timeout: 120000,
  maxBuffer: 64 * 1024 * 1024,
  windowsHide: true,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const lines = result.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, cases.length);
for (let index = 0; index < cases.length; index++) {
  cases[index].expected = JSON.parse(lines[index]);
  assert.equal(cases[index].expected.length, cases[index].steps.length);
}
assert.equal(
  cases[0].expected[2].outcome.length,
  1,
  "Original observer must actually claim a task",
);
await fs.writeFile(
  "tests/migration/scheduler/storage-vectors.json",
  JSON.stringify(
    {
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/scheduler/export-storage.mjs"),
      ),
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  JSON.stringify({
    histories: cases.length,
    transitions: cases.reduce((sum, entry) => sum + entry.steps.length, 0),
  }),
);
