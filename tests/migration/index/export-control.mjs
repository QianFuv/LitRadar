/** Freeze original control migration and lease/checkpoint state-machine observations. */
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
const requests = [];
/** Add one independent database history with optional pre-migration state. */
function scenario(name, operations, setup = "") {
  requests.push({ op: "control", name, setup, operations });
}
const prepare = { op: "prepare" },
  advance = { op: "advance", checkpoint: "opaque:1" },
  complete = { op: "complete", anchor: "anchor:1" };
scenario("empty", [
  { op: "anchor" },
  { op: "checkpoint" },
  { op: "counts" },
  { op: "adopt" },
]);
scenario("resume-keeps-epoch", [
  prepare,
  advance,
  { ...prepare, run: "run-2", timestamp: "later" },
  { op: "checkpoint" },
  { ...advance, run: "run-1" },
  { ...complete, run: "run-2" },
  { op: "anchor" },
  prepare,
  { op: "counts" },
]);
for (const mode of ["bootstrap", "incremental", "full_rescan"]) {
  scenario(`next-batch-${mode}`, [
    prepare,
    complete,
    { ...prepare, batch: "batch-2", mode },
    { op: "checkpoint" },
  ]);
  scenario(`resume-mode-${mode}`, [
    prepare,
    advance,
    { ...prepare, mode },
    { op: "checkpoint" },
    { ...prepare, mode, resume: false },
    { op: "checkpoint" },
  ]);
}
scenario("batch-mismatch", [
  prepare,
  { ...prepare, batch: "other" },
  { ...advance, batch: "other" },
  { ...complete, batch: "other" },
]);
scenario("wrong-base", [
  prepare,
  complete,
  { ...prepare, batch: "batch-2" },
  { ...advance, batch: "batch-2", base: "bad" },
  { ...complete, batch: "batch-2", base: "bad" },
  { op: "checkpoint" },
  { ...complete, batch: "batch-2", base: "anchor:1", anchor: null },
  { op: "anchor" },
]);
scenario("anchor-changed-behind-run", [
  prepare,
  complete,
  { ...prepare, batch: "batch-2" },
  {
    op: "sql",
    sql: "UPDATE provider_sync_anchors SET committed_anchor='unexpected'",
  },
  { ...prepare, batch: "batch-2" },
]);
scenario("lease-boundaries", [
  { op: "acquire", now: 1000 },
  { op: "acquire", run: "other", now: 1299 },
  { op: "heartbeat", run: "other", now: 1001 },
  { op: "heartbeat", now: 1300 },
  { op: "acquire", run: "other", now: 1300 },
  { op: "release" },
  { op: "release", run: "other" },
  { op: "release", run: "other" },
]);
scenario("lease-same-owner-and-release-expired", [
  { op: "acquire", now: 1000 },
  { op: "acquire", now: 999 },
  { op: "heartbeat", now: 1000 },
  { op: "release" },
]);
scenario("progress-needs-no-lease", [
  prepare,
  advance,
  complete,
  { op: "anchor" },
]);
scenario("abandon-is-batch-scoped", [
  prepare,
  { ...prepare, journal: "catalog:second", batch: "batch-2" },
  { op: "acquire", now: 1000 },
  { op: "abandon" },
  { op: "checkpoint" },
  { op: "checkpoint", journal: "catalog:second" },
  { op: "aliases", aliases: ["catalog:second"] },
  { op: "aliases", aliases: ["catalog:missing"] },
]);
for (const batch of [
  "",
  " ",
  "x".repeat(512),
  "x".repeat(513),
  "x\u0000",
  "x\u0085",
  "中".repeat(171),
])
  scenario(`batch-${requests.length}`, [
    { ...prepare, batch },
    { op: "counts", batch },
    { op: "adopt", batch },
    { op: "abandon", batch },
  ]);
for (const value of [
  "",
  " ",
  "x".repeat(65536),
  "x".repeat(65537),
  "中".repeat(21846),
])
  scenario(`opaque-${requests.length}`, [
    prepare,
    { ...advance, checkpoint: value },
    { op: "checkpoint" },
    { ...complete, anchor: value },
    { op: "anchor" },
  ]);
for (const field of ["run", "timestamp"])
  scenario(`empty-${field}`, [
    { ...prepare, [field]: "" },
    prepare,
    { ...advance, [field]: "" },
    { ...complete, [field]: "" },
  ]);
const legacyInsert =
  "INSERT INTO provider_run_checkpoints(catalog_name,provider_name,catalog_id,batch_id,run_id,sync_mode,base_anchor,traversal_checkpoint,started_at,updated_at) VALUES('catalog','scholarly','catalog:example',NULL,'legacy','incremental',NULL,'cursor','epoch','updated'); INSERT INTO provider_sync_anchors(catalog_name,provider_name,catalog_id,committed_anchor,completed_at) VALUES('catalog','scholarly','catalog:done','old-anchor','epoch');";
scenario("legacy-adoption", [
  { op: "sql", sql: legacyInsert },
  { op: "adopt" },
  { op: "adopt", allow: true },
  { op: "counts" },
  prepare,
  { op: "checkpoint" },
  { op: "anchor", journal: "catalog:done" },
]);
scenario("legacy-expired-lease-blocks", [
  { op: "sql", sql: legacyInsert },
  { op: "acquire", now: 0 },
  { op: "adopt", allow: true },
  { op: "release" },
  { op: "adopt", allow: true },
]);
scenario("legacy-mode-mismatch", [
  { op: "sql", sql: legacyInsert },
  { op: "adopt", mode: "full_rescan", allow: true },
]);
scenario("legacy-epoch-mismatch", [
  {
    op: "sql",
    sql:
      legacyInsert +
      "INSERT INTO provider_run_checkpoints SELECT catalog_name,provider_name,'catalog:second',batch_id,run_id,sync_mode,base_anchor,traversal_checkpoint,'other',updated_at FROM provider_run_checkpoints;",
  },
  { op: "adopt", allow: true },
]);
for (const version of [-1, 0, 1, 2, 3, 4, 5, 6])
  scenario(`schema-${version}`, [], `PRAGMA user_version=${version}`);
const legacySchema =
  "CREATE TABLE provider_checkpoints(catalog_name TEXT,provider_name TEXT,scope_kind TEXT,scope_key TEXT,checkpoint TEXT,updated_at TEXT,PRIMARY KEY(catalog_name,provider_name,scope_kind,scope_key));";
for (const marker of [
  '{"state":"complete"}',
  '{"state":"complete","extra":1}',
  '{"state":"complete","state":"complete"}',
  '{"state":"continue","checkpoint":"x"}',
  "null",
  "[]",
  '{"state":"complete"} {}',
  ' { "state" : "complete" } ',
]) {
  const setup =
    legacySchema +
    `INSERT INTO provider_checkpoints VALUES('catalog','zjlib_cnki','journal','catalog:example','${marker}','epoch'); INSERT INTO provider_checkpoints VALUES('catalog','cnki','journal','catalog:retired','{"state":"complete"}','epoch'); PRAGMA user_version=1;`;
  scenario(
    `legacy-marker-${requests.length}`,
    [
      { op: "anchor", provider: "zjlib" },
      { op: "anchor", provider: "cnki_oversea", journal: "catalog:retired" },
    ],
    setup,
  );
}
scenario(
  "legacy-current-name-wins",
  [{ op: "anchor", provider: "zjlib" }],
  legacySchema +
    "INSERT INTO provider_checkpoints VALUES('catalog','zjlib_cnki','journal','catalog:example','{\"state\":\"complete\"}','legacy'); INSERT INTO provider_checkpoints VALUES('catalog','zjlib','journal','catalog:example','{\"state\":\"complete\"}','current'); PRAGMA user_version=1;",
);
const observations = [];
for (const input of requests) {
  const result = spawnSync(build.binary, [], {
    input: JSON.stringify(input) + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 15000,
    maxBuffer: 8 * 1024 * 1024,
  });
  assert.equal(result.status, 0, `${input.name}: ${result.stderr}`);
  assert.ifError(result.error);
  observations.push(JSON.parse(result.stdout));
}
await fs.writeFile(
  "tests/migration/index/control-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-control.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust control workflows`);
