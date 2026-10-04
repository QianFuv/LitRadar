/** Freeze independently observed Rust delivery histories, including failure rollback and owner fencing. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
/** Register one isolated, ordered state-machine history. */
function add(name, steps) {
  cases.push({ name, steps });
}
const admit = { op: "admit" };
const claimed = [admit, { op: "claim" }];
const running = [...claimed, { op: "start", revision: 1 }];
const subscriber = { kind: "subscriber", key: "1", user_id: 1 };
const article = { kind: "article", key: "7", article_id: 7 };
const work = [
  ...running,
  { op: "items", items: [subscriber, article] },
  { op: "claim_item", revision: 2 },
  { op: "reserve" },
];
const sending = [...work, { op: "sending", revision: 1 }];
const reservations = [{ id: 1, revision: 0 }];
add("admission-idempotence-and-terminal", [
  admit,
  admit,
  { op: "claim" },
  { op: "start", revision: 1 },
  { op: "finalize", revision: 2 },
  admit,
  { op: "claim", revision: 3 },
]);
add("competing-run-claim", [
  ...claimed,
  { op: "admit", external_id: "second" },
  { op: "claim", run: 2 },
  { op: "claim", run: 1, revision: 1, now: 111, owner: "replacement" },
  { op: "renew", revision: 1, now: 112 },
  { op: "renew", revision: 2, now: 112, owner: "replacement" },
]);
add("lease-monotonic-and-expiry", [
  ...running,
  { op: "acquire" },
  { op: "acquire" },
  { op: "renew_lease", revision: 0 },
  { op: "release_lease", revision: 1, now: 111 },
  { op: "acquire", now: 112 },
  { op: "release_lease", revision: 0, now: 113 },
]);
add("checkpoint-cas-raw-bytes", [
  { op: "checkpoint", snapshot: ' {\n"issue_article_counts":{} }\n' },
  { op: "checkpoint" },
  {
    op: "checkpoint",
    checkpoint_revision: 0,
    snapshot: "[1,2]",
    completed_at: "raw timestamp",
  },
  { op: "checkpoint", checkpoint_revision: 0 },
]);
add("atomic-success", [
  ...sending,
  {
    op: "finalize_attempt",
    revision: 2,
    reservations,
    message: "message",
    result: '{"selected_article_ids":[7]}',
  },
  { op: "acquire" },
  {
    op: "finalize_checkpoint",
    revision: 2,
    lease_revision: 0,
    snapshot: '{"issue_article_counts":{"1:2":1}}',
    completed_at: "completed",
  },
]);
add("atomic-unknown", [
  ...sending,
  {
    op: "finalize_attempt",
    revision: 2,
    reservations,
    status: "unknown",
    dedupe_status: "unknown",
    error_code: "ambiguous_delivery",
  },
  { op: "cleanup", now: 999 },
]);
add("recovery-mixed-items-and-reservations", [
  ...sending,
  { op: "claim_item", item: 2, revision: 2 },
  { op: "reserve", article_id: 8 },
  { op: "items", items: [{ kind: "subscriber", key: "2", user_id: 2 }] },
  { op: "claim_item", item: 3, revision: 2 },
  { op: "reserve", user_id: 2, article_id: 9 },
  { op: "claim", revision: 2, now: 111, owner: "replacement" },
  { op: "reconcile", revision: 3, now: 111, owner: "replacement" },
  {
    op: "finalize_attempt",
    revision: 2,
    reservations,
    message: "stale",
    now: 112,
  },
]);
for (const status of [
  "completed",
  "failed",
  "cancelled",
  "timed_out",
  "skipped",
  "unknown",
  "running",
])
  add(`finalize-${status}`, [
    ...running,
    { op: "finalize", status, revision: 2, now: 200 },
  ]);
for (const status of [
  "completed",
  "failed",
  "cancelled",
  "timed_out",
  "skipped",
  "unknown",
])
  add(`queued-finalize-${status}`, [
    admit,
    { op: "finalize_queued", status, error_code: "fixture" },
  ]);
for (const op of ["start", "renew", "reconcile"]) {
  for (const now of [110.249, 110.25, 110.251])
    add(`${op}-expiry-${now}`, [...claimed, { op, revision: 1, now }]);
}
add("cancel-before-claim", [
  admit,
  { op: "cancel" },
  { op: "claim", revision: 1 },
]);
add("cancel-during-run", [
  ...running,
  { op: "cancel", revision: 2 },
  { op: "renew", revision: 3 },
  { op: "finalize", revision: 4, status: "cancelled" },
]);
add("items-ensure-versus-insert", [
  ...running,
  { op: "items", items: [subscriber, article] },
  { op: "items", items: [subscriber, article] },
  { op: "insert_items", items: [subscriber, article] },
  { op: "items", items: [{ ...article, article_id: 8 }] },
  { op: "claim_next", revision: 2 },
  { op: "finalize_item", revision: 1 },
  { op: "claim_next", revision: 2 },
  { op: "finalize_item", item: 2, revision: 1 },
  { op: "claim_next", revision: 2 },
]);
add("duplicate-item-input-atomic", [
  ...running,
  { op: "items", items: [article, article] },
]);
add("duplicate-reservations-atomic", [
  ...sending,
  {
    op: "finalize_attempt",
    revision: 2,
    reservations: [...reservations, ...reservations],
    message: "message",
  },
]);
add("reservation-release-atomic", [
  ...work,
  { op: "reserve", article_id: 8 },
  {
    op: "release_reservations",
    reservations: [
      { id: 1, revision: 0 },
      { id: 2, revision: 1 },
    ],
  },
  {
    op: "release_reservations",
    reservations: [
      { id: 1, revision: 0 },
      { id: 2, revision: 0 },
    ],
  },
]);
add("dedupe-existing-and-cleanup-boundary", [
  ...work,
  { op: "reserve" },
  { op: "resolve", message: "message", now: 101 },
  { op: "cleanup", now: 101 },
  { op: "cleanup", now: 101.001 },
]);
for (const message of ["", " ", "\n", "x".repeat(257)])
  add(`message-${JSON.stringify(message)}`, [
    ...sending,
    { op: "finalize_attempt", revision: 2, reservations, message },
  ]);
for (const op of ["finalize_attempt", "finalize_checkpoint"]) {
  const base =
    op === "finalize_attempt" ? sending : [...running, { op: "acquire" }];
  const final =
    op === "finalize_attempt"
      ? { op, revision: 2, reservations, message: "message" }
      : { op, revision: 2, lease_revision: 0 };
  const table =
    op === "finalize_attempt" ? "delivery_run_items" : "delivery_runs";
  for (const action of ["abort", "delete"]) {
    const sql =
      action === "abort"
        ? `CREATE TRIGGER fault BEFORE UPDATE ON ${table} BEGIN SELECT RAISE(ABORT,'fixture failure'); END;`
        : `CREATE TRIGGER fault AFTER UPDATE ON ${table} BEGIN DELETE FROM ${table} WHERE id=NEW.id; END;`;
    add(`${op}-${action}-rollback`, [...base, { ...final, sql }]);
  }
  add(`${op}-wrong-revision`, [...base, { ...final, revision: 99 }]);
}
for (const op of ["claim_next", "claim_item"])
  add(`${op}-corrupt-old-owner`, [
    ...running,
    { op: "items", items: [subscriber] },
    {
      op,
      revision: 2,
      sql: "PRAGMA ignore_check_constraints=ON; UPDATE delivery_run_items SET owner_id=X'ff'; PRAGMA ignore_check_constraints=OFF;",
    },
  ]);
for (const [table, op, field] of [
  ["delivery_runs", "load", "status"],
  ["delivery_run_items", "list_items", "status"],
  ["delivery_checkpoints", "load_checkpoint", "status"],
  ["delivery_leases", "load_lease", "owner_id"],
]) {
  const base = [
    ...running,
    { op: "items", items: [subscriber] },
    { op: "checkpoint" },
    { op: "acquire" },
  ];
  add(`corrupt-${table}`, [
    ...base,
    {
      op,
      sql: `PRAGMA ignore_check_constraints=ON; UPDATE ${table} SET ${field}=X'ff'; PRAGMA ignore_check_constraints=OFF;`,
    },
  ]);
}
const manual = {
  op: "admit_manual",
  trigger: "manual",
  db_name: null,
  scope_key: "manual:1",
  user_id: 1,
  deadline: 700,
};
add("manual-busy-and-unknown-quarantine", [
  manual,
  { ...manual, external_id: "other" },
  { op: "claim" },
  { op: "start", revision: 1 },
  { op: "finalize", revision: 2, status: "unknown" },
  { ...manual, external_id: "replacement" },
]);
for (const [field, values] of Object.entries({
  owner: ["", "x".repeat(129), "with space"],
  seconds: [0, -1],
  revision: [-1, 99],
  run: [0, 99],
  now: [-1],
}))
  for (const value of values)
    add(`claim-invalid-${field}-${String(value)}`, [
      admit,
      { op: "claim", [field]: value },
    ]);

const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/delivery-oracle/durable-build.json",
    "utf8",
  ),
);
assert.equal(digest(await fs.readFile(build.binary)), build.binary_sha256);
assert.equal(
  digest(await fs.readFile("tests/migration/delivery/build-durable.mjs")),
  build.builder_sha256,
);
for (const input of [...build.inputs, ...build.dependencies])
  assert.equal(digest(await fs.readFile(input.path)), input.sha256, input.path);
const parent = path.resolve("output/migration/execution/delivery-oracle");
const temporary = await fs.mkdtemp(path.join(parent, "durable-cases-"));
const inputs = cases.map((item, index) => ({
  ...item,
  path: path.join(temporary, String(index), "auth.sqlite"),
}));
const result = spawnSync(build.binary, [], {
  input: inputs.map((item) => JSON.stringify(item)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 90000,
  maxBuffer: 64 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const lines = result.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, cases.length);
await fs.writeFile(
  "tests/migration/delivery/durable-vectors.json",
  JSON.stringify(
    {
      schema: 1,
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/delivery/export-durable.mjs"),
      ),
      cases: cases.map((item, index) => ({
        ...item,
        output: JSON.parse(lines[index]),
      })),
    },
    null,
    2,
  ) + "\n",
);
assert.equal(path.dirname(path.resolve(temporary)), parent);
await fs.rm(temporary, { recursive: true, force: true });
console.log(
  `Frozen ${cases.length} original durable histories (${cases.reduce((sum, item) => sum + item.steps.length, 0)} transitions)`,
);
