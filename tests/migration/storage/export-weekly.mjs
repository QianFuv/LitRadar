/** Export original weekly parser observations without deriving expectations from the Go port. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const dates = [
  "2026-10-03T12:34:56Z",
  "2026-10-03t12:34:56z",
  "2026-10-03 12:34:56Z",
  "2016-12-31T23:59:60Z",
  "2026-10-03T12:34:60.1234567891+08:00",
  "2026-10-03T12:34:56−08:00",
  "0000-01-01T00:00:00+23:59",
  "9999-12-31T23:59:59-23:59",
  "2024-02-29T00:00:00Z",
  "2026-02-29T00:00:00Z",
  "2026-10-03T24:00:00Z",
  "2026-10-03T00:00:61Z",
  "2026-10-03T00:00:00+24:00",
  "2026-10-03T00:00:00+23:59",
  "2026-10-03T00:00:00+00:60",
  "2026-10-03T00:00:00.1Z",
  "2026-10-03T00:00:00.000001Z",
  "2026-10-03T00:00:00.1234567899Z",
  "2026-10-03T00:00:00.Z",
  "2026-10-03T00:00:00,1Z",
  "2026-10-03T00:00:00",
  "2026-10-03T00:00:00+0800",
  "\u0085 2026-10-03T00:00:00Z\u3000",
  "0",
  "-1",
  "01",
  "+1",
  "-0",
  "9223372036854775807",
  "-8334601228800",
  "8210266876799",
  "",
];
const requests = dates.map((input) => ({ kind: "date", input }));
const base = {
  db_name: "catalog.sqlite",
  generated_at: "2026-10-03T12:34:56Z",
  run_id: "source",
  notifiable_article_ids: [1, 2, 1, -1, 0],
};
for (const generated_at of dates)
  requests.push({
    kind: "manifest",
    input: JSON.stringify({ ...base, generated_at }),
  });
for (const input of [
  JSON.stringify(base),
  JSON.stringify({ ...base, generated_at: null, run_id: "0" }),
  JSON.stringify({ ...base, generated_at: "", run_id: "0" }),
  '["catalog.sqlite","0",null,[1,2]]',
  '["catalog.sqlite","0",null]',
  '["catalog.sqlite","0"]',
  '["catalog.sqlite","0",null,[1],0]',
  '{"db_name":"catalog.sqlite","run_id":"0","notifiable_article_ids":[0,-0,1,1.0,1e0,-1,-9223372036854775808,9223372036854775807,9223372036854775808,18446744073709551615,"1",true,null,{},[]]}',
  ...[null, {}, "ids", 1].map((notifiable_article_ids) =>
    JSON.stringify({ ...base, notifiable_article_ids }),
  ),
  JSON.stringify({ ...base, db_name: null }),
  JSON.stringify({ ...base, db_name: 1 }),
  JSON.stringify({ ...base, generated_at: 1 }),
  JSON.stringify({ ...base, run_id: [] }),
  JSON.stringify(base).replace('"db_name":', '"db_name":"other", "db_name":'),
  JSON.stringify(base).replace('"db_name":', '"DB_NAME":"ignored", "db_name":'),
  JSON.stringify({ ...base, ignored: [1, 2, { a: true }] }),
  JSON.stringify(base) + " {}",
  JSON.stringify(base) + " trailing",
  JSON.stringify(base).replace('"run_id":"source"', '"run_id":"\\ud800"'),
  JSON.stringify(base).replace(
    '"run_id":"source"',
    '"ignored":"\\ud800","run_id":"source"',
  ),
  "{}",
  "null",
  "[]",
])
  requests.push({ kind: "manifest", input });
for (const db_name of [
  "",
  " ",
  ".",
  "..",
  "/",
  "a/../b",
  "folder/catalog",
  "folder/catalog/.",
  "catalog.SQLITE",
  "C:\\folder\\catalog",
  "\\\\server\\share\\catalog",
])
  requests.push({
    kind: "manifest",
    input: JSON.stringify({ ...base, db_name }),
  });
const result = spawnSync("output/migration/execution/weekly-oracle.exe", [], {
  cwd: WORKSPACE_ROOT,
  input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
  encoding: "utf8",
  shell: false,
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 8 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const cases = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(cases.length, requests.length);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/weekly_manifest.rs",
  "tests/migration/storage/weekly-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/weekly-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, platform: process.platform, sources, cases },
    null,
    2,
  ) + "\n",
);
console.log(
  `Exported ${cases.length} original Rust weekly parsing observations`,
);
