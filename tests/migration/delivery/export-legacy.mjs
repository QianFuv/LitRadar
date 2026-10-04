/** Freeze original legacy-import results, including atomic rollback and byte-hash identity. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
/** Register an entire import history with exact input file bytes. */
function add(name, bodies, options = {}) {
  cases.push({
    name,
    steps: bodies.map((body) => ({
      files: [
        {
          path: "data/push_state/fixture.json",
          body: typeof body === "string" ? body : JSON.stringify(body),
        },
      ],
      now: 100.25,
      ...options,
    })),
  });
}
const basic = { db_name: "fixture.sqlite" };
add("fresh-and-idempotent", [basic, basic]);
add("raw-byte-conflict", [basic, JSON.stringify(basic) + "\n"]);
for (const status of [
  "",
  "idle",
  "running",
  "claimed",
  "completed",
  "failed",
  "skipped",
  "unknown",
  "timed_out",
  "custom",
  " completed ",
]) {
  add(`statuses-${status}`, [
    {
      ...basic,
      status,
      run: {
        run_id: "legacy-run",
        status,
        pending_issue_keys: ["+1:02"],
        done_issue_keys: ["1:3"],
        pending_inpress_keys: ["01"],
        done_inpress_keys: ["2"],
        delivered_article_ids: [12],
        user_results: [
          {
            subscriber_id: "1",
            selected_count: 3,
            pushed_count: 2,
            folder_synced_count: 1,
            status,
          },
        ],
      },
      delivery_dedupe: { "1:12": "original-date" },
    },
  ]);
}
for (const [name, body] of Object.entries({
  "sequence-minimal": '["fixture.sqlite"]',
  "sequence-complete":
    '["fixture.sqlite","completed",null,[{"+1:02":3},{"01":2}],["run","completed",[],[],[],[],[],[["1",2,1,null,"ok"]]],{"1:2":"date"}]',
  "unknown-fields": '{"db_name":"fixture.sqlite","extra":{"x":[1,true]}}',
  "unknown-huge-number": '{"db_name":"fixture.sqlite","extra":1e999}',
  "unknown-lone-surrogate": '{"db_name":"fixture.sqlite","extra":"\\ud800"}',
  "duplicate-known":
    '{"db_name":"fixture.sqlite","status":"idle","status":"completed"}',
  "duplicate-map":
    '{"db_name":"fixture.sqlite","snapshot":{"issue_article_counts":{"1:2":1,"1:2":2}}}',
  "duplicate-map-bad-first":
    '{"db_name":"fixture.sqlite","snapshot":{"issue_article_counts":{"1:2":"bad","1:2":2}}}',
  empty: "",
  null: "null",
  "array-empty": "[]",
  "wrong-name": '{"db_name":"other.sqlite"}',
  "signed-zero-count":
    '{"db_name":"fixture.sqlite","snapshot":{"issue_article_counts":{"1:2":-0}}}',
  "float-count":
    '{"db_name":"fixture.sqlite","snapshot":{"issue_article_counts":{"1:2":1.0}}}',
  "huge-count":
    '{"db_name":"fixture.sqlite","run":{"run_id":"r","user_results":[{"subscriber_id":"1","selected_count":18446744073709551615}]}}',
  "overflow-count":
    '{"db_name":"fixture.sqlite","run":{"run_id":"r","user_results":[{"subscriber_id":"1","selected_count":18446744073709551616}]}}',
}))
  add(name, [body]);
for (const field of ["status", "snapshot", "delivery_dedupe"])
  add(`null-${field}`, [{ ...basic, [field]: null }]);
for (const field of [
  "status",
  "pending_issue_keys",
  "done_issue_keys",
  "pending_inpress_keys",
  "done_inpress_keys",
  "delivered_article_ids",
  "user_results",
])
  add(`run-null-${field}`, [{ ...basic, run: { run_id: "r", [field]: null } }]);
for (const [name, extra] of Object.entries({
  "cross-list-conflict": {
    run: { run_id: "r", pending_issue_keys: ["1:2"], done_issue_keys: ["1:2"] },
  },
  "duplicate-progress": {
    run: { run_id: "r", pending_issue_keys: ["1:2", "1:2"] },
  },
  "duplicate-subscriber": {
    run: {
      run_id: "r",
      user_results: [{ subscriber_id: "1" }, { subscriber_id: "1" }],
    },
  },
  "missing-user": {
    run: { run_id: "r", user_results: [{ subscriber_id: "99" }] },
  },
  "negative-article": { run: { run_id: "r", delivered_article_ids: [-1] } },
  "bad-key": { snapshot: { issue_article_counts: { "1:0": 1 } } },
  "negative-count": { snapshot: { inpress_article_counts: { 1: -1 } } },
  "noncanonical-keys": {
    snapshot: {
      issue_article_counts: { "+1:02": 1 },
      inpress_article_counts: { "01": 2 },
    },
  },
  "dedupe-canonical-collision": {
    delivery_dedupe: { "1:02": "a", "+1:2": "b" },
  },
  "empty-timestamp": { last_completed_run_at: "" },
}))
  add(name, [{ ...basic, ...extra }]);
cases.push({
  name: "all-files-parse-before-write",
  steps: [
    {
      files: [
        {
          path: "data/push_state/first.json",
          body: '{"db_name":"first.sqlite"}',
        },
        { path: "data/push_state/second.json", body: "invalid" },
        { path: "data/push_state/ignored.changes.json", body: "invalid" },
      ],
    },
  ],
});
cases.push({
  name: "cross-workflow-atomic-sql-failure",
  steps: [
    {
      files: [
        { path: "data/push_state/fixture.json", body: JSON.stringify(basic) },
        {
          path: "data/folder_push_state/fixture.json",
          body: JSON.stringify({
            ...basic,
            delivery_dedupe: { "99:1": "date" },
          }),
        },
      ],
    },
  ],
});
add("existing-nonlegacy-checkpoint", [basic], {
  sql: "INSERT INTO delivery_checkpoints(workflow,db_name,status,snapshot_json,revision,created_at,updated_at) VALUES('notify','fixture.sqlite','idle','{}',0,1,1)",
});
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/delivery-oracle/legacy-build.json",
    "utf8",
  ),
);
assert.equal(digest(await fs.readFile(build.binary)), build.binary_sha256);
const parent = path.resolve("output/migration/execution/delivery-oracle");
const temporary = await fs.mkdtemp(path.join(parent, "legacy-cases-"));
assert(path.dirname(temporary) === parent);
const inputs = cases.map((item, index) => ({
  ...item,
  root: path.join(temporary, String(index)),
}));
const result = spawnSync(build.binary, [], {
  input: inputs.map((item) => JSON.stringify(item)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 90000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const lines = result.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, cases.length);
await fs.writeFile(
  "tests/migration/delivery/legacy-vectors.json",
  JSON.stringify(
    {
      schema: 1,
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/delivery/export-legacy.mjs"),
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
assert(path.dirname(path.resolve(temporary)) === parent);
await fs.rm(temporary, { recursive: true, force: true });
console.log(`Frozen ${cases.length} original legacy import histories`);
