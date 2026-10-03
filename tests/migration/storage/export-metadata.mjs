/** Export original metadata query behavior from original fixture SQL and synthetic edge rows. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const original = await fs.readFile(
  "crates/litradar-storage/src/index/test_support.rs",
  "utf8",
);
const sql = original
  .slice(original.indexOf("pub(super) fn create_fixture_schema"))
  .match(/r#"([\s\S]*?)"#/)[1];
const additional = `
UPDATE journals SET abs_rating='4*',fms_rating='A' WHERE journal_id=1;
UPDATE journals SET abs_rating='4',fms_rating='B' WHERE journal_id=2;
UPDATE journals SET abs_rating='4*',fms_rating='A',area='  ',title_aliases_json='[["Historical Alias"]]',issns_json='[{"display_name":"2434-561X"}]' WHERE journal_id=3;
INSERT INTO issues(issue_id,journal_id,publication_year,title,date) VALUES
 (30,3,2027,'Future empty issue',' 2027 '),(31,3,NULL,NULL,NULL),(32,3,0,'Year zero','0000-02-29'),
 (33,3,2026,'Invalid date','2026-02-30'),(34,3,2026,'Month','2026-02');
INSERT INTO articles(article_id,journal_id,title,date,authors_json,open_access,in_press) VALUES
 (1010,2,'Undated',NULL,'[]',NULL,NULL),(1011,2,'Empty date','','[["Sequence author"]]',2,-1),
 (1012,2,'Café 科技金融','2026-02-01','[]',0,0);
INSERT INTO article_listing(article_id,journal_id,date,open_access,in_press,area)
 SELECT article_id,journal_id,date,open_access,in_press,'Stale projection area' FROM articles WHERE article_id>=1010;
INSERT INTO article_search(rowid,article_id,title) VALUES (1010,1010,'Undated'),(1011,1011,'Empty date'),(1012,1012,'Cafe 科技金融');
`;
const temporary = await fs.mkdtemp(
  path.join(WORKSPACE_ROOT, "output/migration/execution/metadata-oracle-"),
);
const sqlPath = path.join(temporary, "input.sql");
await fs.writeFile(sqlPath, sql + additional);
const requests = [];
for (const operation of ["areas", "ratings", "options", "years"])
  requests.push({ operation });
for (const id of [1, 2, 3, -1, 999])
  requests.push({ operation: "journal", id });
for (const id of [10, 30, 31, 32, 33, 34, -1, 999])
  requests.push({ operation: "issue", id });
for (const params of [
  { limit: 50 },
  { limit: 1, offset: 1 },
  { limit: 50, has_articles: false },
  { limit: 50, has_articles: true },
  { limit: 50, year: 2027 },
  { limit: 50, year: 2027, has_articles: true },
  { limit: 50, area: " Medicine " },
  { limit: 50, area: " " },
  { limit: 50, ratings: { abs_rating: [" 4* ", "4*"] } },
  { limit: 50, ratings: { abs_rating: ["4*"], fms_rating: ["A"] } },
  { limit: 50, ratings: { abs_rating: ["4*"], fms_rating: ["B"] } },
  { limit: 0 },
  { limit: 201 },
  { limit: 1, offset: -1 },
  { limit: 50, ratings: { abs_rating: [""] } },
  { limit: 50, ratings: { abs_rating: [" "] } },
  { limit: 50, ratings: { abs_rating: ["x".repeat(2049)] } },
  { limit: 50, ratings: { abs_rating: Array(501).fill("4*") } },
  { limit: 50, area: "", ratings: { abs_rating: Array(500).fill("4*") } },
  ...[
    "",
    " , ",
    "-title",
    "title:DESC, journal_id",
    "title:nonsense",
    "-title:asc",
    "Title",
    "title:deſc",
  ].map((sort) => ({ limit: 50, sort })),
])
  requests.push({ operation: "journals", params });
for (const params of [
  { limit: 50 },
  { limit: 2, offset: 2 },
  { limit: 50, journal_id: 3 },
  { limit: 50, year: 2026 },
  { limit: 50, journal_id: 3, year: 2026 },
  { limit: 50, journal_id: 0 },
  { limit: 50, year: 0 },
  { limit: 0 },
  { limit: 50, offset: -1 },
  ...[
    "",
    " , ",
    "-issue_id",
    "date:asc",
    "title:bad",
    "journal_id",
    "-title:desc",
  ].map((sort) => ({ limit: 50, sort })),
])
  requests.push({ operation: "issues", params });
requests.push(
  {
    operation: "journals",
    db: "missing.sqlite",
    params: { limit: 0, ratings: { abs_rating: [""] } },
  },
  {
    operation: "journals",
    db: "missing.sqlite",
    params: { limit: 50, ratings: { abs_rating: [""] } },
  },
);
for (const id of [1001, 1002, 1003, 1004, 1010, 1011, 1012, -1, 999])
  requests.push({ operation: "article", id });
for (const params of [
  {},
  { limit: 2 },
  { limit: 2, offset: 2 },
  { limit: 2, include_total: false },
  { limit: 2, cursor: "2026-02-01|1012" },
  { limit: 2, cursor: "2026-02-01|1012", offset: 10, include_total: true },
  { limit: 2, sort: "date:asc" },
  { limit: 2, sort: "date:asc", cursor: "|1010" },
  { limit: 2, cursor: "|1010" },
  { limit: 50, cursor: "2026-02-01|+1012" },
  { journal_id: [1, 1], issue_id: 10 },
  { area: ["Medicine"] },
  { area: ["Stale projection area"] },
  { ratings: { abs_rating: ["4"] }, area: ["Engineering"] },
  { ratings: { abs_rating: ["4"] }, area: ["Stale projection area"] },
  { ratings: { abs_rating: ["4*"] }, journal_id: [2] },
  { ratings: { abs_rating: ["missing"] } },
  { in_press: true },
  { in_press: false },
  { open_access: true },
  { open_access: false },
  { date_from: " 2026-01-04 ", date_to: "2026-01-05" },
  { doi: " 10.1000/genome " },
  { pmid: "1002" },
  { year: 2026 },
  ...[
    "Genome",
    "genome OR Clinical",
    "indexedonly",
    "科技金融",
    "kejijinrong",
    "Café",
    "ÓR",
    "\u0301",
    '"',
    "title:Genome NOT preview",
    "missing:Genome",
  ].flatMap((q) =>
    ["simple", "advanced"].map((search_mode) => ({ q, search_mode })),
  ),
  ...[
    "",
    ",",
    "date:asc",
    "date:bogus",
    "date:deſc",
    "-date",
    "-date:asc",
    "title",
    "date,date",
    "DATE",
  ].map((sort) => ({ sort })),
  ...[
    "invalid",
    "2026|",
    "2026|1|2",
    "2026|9223372036854775808",
    "2026| 1",
    "2026|١",
  ].map((cursor) => ({ cursor })),
  {
    ratings: { abs_rating: ["missing"] },
    cursor: "invalid",
    q: '"',
    search_mode: "advanced",
  },
  {
    ratings: { abs_rating: ["missing"] },
    cursor: "|0",
    q: '"',
    search_mode: "advanced",
  },
  { ratings: { abs_rating: ["missing"] }, q: '"', search_mode: "simple" },
  { include_total: false, q: '"', search_mode: "advanced" },
  { limit: 0 },
  { limit: 201 },
  { offset: -1 },
  { journal_id: Array(501).fill(1) },
  { area: ["x".repeat(2049)] },
  { q: "x".repeat(2049) },
  { ratings: { abs_rating: [" "] } },
])
  requests.push({ operation: "articles", params });
requests.push({ operation: "issue_counts" }, { operation: "inpress_counts" });
for (const keys of [
  [],
  ["1:10"],
  ["999:10"],
  ["1:10", "1:10", "2:20"],
  ["-1:-1"],
  ["+1:+10"],
  ["invalid"],
  ["1:10:1"],
  ["1:9223372036854775808"],
])
  requests.push({ operation: "issue_candidates", keys });
for (const keys of [
  [],
  ["1"],
  ["1", "1", "2"],
  ["+1"],
  ["-1"],
  [" 1"],
  ["9223372036854775808"],
])
  requests.push({ operation: "inpress_candidates", keys });
for (const ids of [[], [1001], [1001, 1001, 1010, 1011, 1012, -1], [999]])
  requests.push({ operation: "article_candidates", ids });
const result = spawnSync(
  "output/migration/execution/metadata-oracle.exe",
  [temporary, sqlPath],
  {
    cwd: WORKSPACE_ROOT,
    input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const cases = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(cases.length, requests.length);
const fixture = await fs.readFile(
  path.join(temporary, "data/index/metadata.sqlite"),
);
await fs.writeFile(
  "tests/migration/storage/fixtures/metadata.sqlite.fixture",
  fixture,
);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/index/metadata.rs",
  "crates/litradar-storage/src/index/articles.rs",
  "crates/litradar-storage/src/index/shared.rs",
  "crates/litradar-storage/src/index/test_support.rs",
  "crates/litradar-domain/src/index_contract.rs",
  "tests/migration/storage/metadata-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/metadata-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, sources, fixture_sha256: digest(fixture), cases },
    null,
    2,
  ) + "\n",
);
console.log(`Exported ${cases.length} unchanged Rust metadata observations`);
