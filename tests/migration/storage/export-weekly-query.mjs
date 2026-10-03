/** Freeze original fixed-clock weekly queries without changing their business code. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const end = "2026-10-03T12:00:00.123456789Z";
const publication = (
  ids,
  run = null,
  date = "2026-10-02T12:00:00.1Z",
  db = "metadata.sqlite",
) =>
  JSON.stringify({
    db_name: db,
    generated_at: date,
    run_id: run,
    notifiable_article_ids: ids,
  });
const normal = {
  "a.changes.json": publication([1012, 1001, 1011, 1010, 999999], "  latest  "),
  "b.changes.json": publication(
    [1001, 1002],
    null,
    "2026-09-26T12:00:00.123456789Z",
  ),
  "old.changes.json": publication(
    [1003],
    "old",
    "2026-09-26T12:00:00.123456788Z",
  ),
  "future.changes.json": publication(
    [1003],
    "future",
    "2026-10-03T12:00:00.123456790Z",
  ),
};
const cases = [];
for (const operation of ["summary", "legacy", "available"]) {
  for (const manifests of [
    {},
    normal,
    { "a.changes.json": publication([999999]) },
    { "a.changes.json": publication([1001], null, undefined, "absent.sqlite") },
  ])
    cases.push({ operation, end, selected: [], manifests });
  for (const sql of [
    "DELETE FROM article_listing WHERE article_id=1001;",
    "DELETE FROM articles WHERE article_id=1012;",
    "PRAGMA foreign_keys=OFF; UPDATE article_listing SET journal_id=999 WHERE article_id=1012;",
  ])
    cases.push({ operation, end, selected: [], manifests: normal, sql });
}
for (const run of [null, "", "  ", " run "]) {
  for (const date of [
    "2026-10-02T12:00:00.1Z",
    "2026-10-02T12:00:00.000001Z",
    "2026-10-02T12:00:00.123456789Z",
    "2026-10-02T12:00:60.25Z",
    "2026-10-02T20:00:00+08:00",
  ])
    cases.push({
      operation: "available",
      end,
      selected: [],
      manifests: {
        "a.changes.json": publication([1012, 1001, 1012, 999999], run, date),
      },
    });
}
for (const selected of [["metadata.sqlite"], ["metadata"], ["absent.sqlite"]])
  cases.push({ operation: "available", end, selected, manifests: normal });
cases.push({
  operation: "available",
  end,
  selected: [],
  manifests: {
    "a.changes.json": publication([1001], null),
    "b.changes.json": publication([1001], " "),
  },
});
for (const params of [
  {},
  { journal_id: 1 },
  { limit: 1 },
  { limit: 1, cursor: "2026-02-01|1012" },
  { cursor: "|1011" },
  { cursor: "bad" },
  { q: "科技" },
  { q: "Cafe" },
  { q: '"' },
  { q: "missing" },
  { q: " " },
  { journal_id: 999 },
  { journal_id: 0 },
  { limit: 0 },
  { limit: 201 },
  { window_end: "0" },
  { window_end: "bad" },
  { db: " " },
  { db: "absent" },
  { db: "some/path/metadata" },
  { q: "x".repeat(2049) },
  { cursor: "x".repeat(2049) },
])
  cases.push({
    operation: "page",
    end,
    manifests: normal,
    params: { db: "metadata", journal_id: 2, limit: 50, ...params },
  });
for (const operation of ["legacy", "summary", "available"])
  cases.push({
    operation,
    end,
    selected: [],
    manifests: {
      "many.changes.json": publication(
        Array.from({ length: 2001 }, (_, index) => index + 5000),
        null,
        undefined,
        "absent.sqlite",
      ),
    },
  });
for (const journal_id of [999, 2])
  cases.push({
    operation: "page",
    end,
    manifests: { "bad.changes.json": "{" },
    params: { db: "metadata", journal_id, limit: 50 },
    error_category: journal_id === 2,
  });
for (const operation of ["summary", "legacy", "page"])
  cases.push({
    operation,
    end,
    manifests: normal,
    params: { db: "metadata", journal_id: 2, limit: 50 },
    sql: "UPDATE articles SET pmid=X'ff' WHERE article_id=1012",
    error_category: operation === "page",
  });
const temporary = await fs.mkdtemp(
  path.join(WORKSPACE_ROOT, "output/migration/execution/weekly-query-oracle-"),
);
const fixturePath = "tests/migration/storage/fixtures/metadata.sqlite.fixture";
const result = spawnSync(
  "output/migration/execution/weekly-query-oracle.exe",
  [temporary, fixturePath],
  {
    cwd: WORKSPACE_ROOT,
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 32 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/t04-weekly-query-oracle-build.json",
    "utf8",
  ),
);
const sources = [];
for (const filename of [
  "crates/litradar-storage/src/weekly_manifest.rs",
  "crates/litradar-storage/src/article_authors.rs",
  "tests/migration/storage/weekly-query-oracle.rs",
  "tests/migration/storage/build-oracles.mjs",
  "tests/migration/storage/export-weekly-query.mjs",
  fixturePath,
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/weekly-query-vectors.json",
  JSON.stringify(
    {
      baseline: BASELINE,
      sources,
      visibility_only_sources: build.copiedSources,
      cases: observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Exported ${observations.length} original fixed-clock weekly observations`,
);
