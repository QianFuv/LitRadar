/** Export original owned-folder reads, cursor grammar and metadata-failure classification. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const temporary = await fs.mkdtemp(
  path.join(WORKSPACE_ROOT, "output/migration/execution/favorites-oracle-"),
);
const sql = `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'owner','hash','salt',1,1),(2,'other','hash','salt',2,2);
INSERT INTO folders(id,user_id,name,is_tracking,created_at,updated_at) VALUES(10,1,'first',1,10.25,10.25),(11,1,'second',0,11,11),(20,2,'private',1,12,12);
INSERT INTO favorites(id,user_id,folder_id,article_id,db_name,note,created_at) VALUES
(1,1,10,1001,'metadata','one',12.25),(2,1,10,1012,'metadata','two',12.25),(3,1,10,999,'metadata','gone',13),
(4,1,11,1001,'metadata','second folder',14),(5,2,20,1001,'metadata','private',15),(6,1,10,1011,'','default',9);`;
const sqlPath = path.join(temporary, "seed.sql");
await fs.writeFile(sqlPath, sql);
const requests = [];
const encode = (text) => Buffer.from(text).toString("base64url");
for (const operation of [
  "folders",
  "tracking",
  "count",
  "list",
  "page",
  "snapshot",
]) {
  for (const owner of [1, 2, 99]) requests.push({ operation, owner });
}
for (const limit of [0, -1, 1, 2, 3, 500, 501])
  requests.push({ operation: "page", folder: 10, limit });
for (const limit of [0, 1, 4, 10])
  requests.push({ operation: "snapshot", folder: 10, limit });
for (const cursor of [
  null,
  "",
  encode("1|1|10|4028800000000000|2"),
  encode("1|+1|010|+4028800000000000|+2"),
  encode("1|1|10|8000000000000000|2"),
  encode("1|1|10|0|2"),
  encode("1|1|10|7ff0000000000000|2"),
  encode("1|1|10|7ff8000000000000|2"),
  encode("1|1|10|bff0000000000000|2"),
  encode("1|1|10|0x0|2"),
  encode("1|1|10|0|0"),
  encode("2|1|10|0|2"),
  encode("1|2|10|0|2"),
  encode("1|1|11|0|2"),
  encode("1|1|10|0|2") + "=",
  encode("1|1|10|0|2") + "\n",
  encode("1|1|10|00000000000000000000000000|2"),
  "A".repeat(129),
])
  requests.push({ operation: "page", cursor, limit: 2 });
requests.push(
  { operation: "page", owner: 2, folder: 10, cursor: "invalid" },
  { operation: "page", folder: 0, cursor: "invalid" },
);
for (const operation of ["list", "page"]) {
  requests.push(
    { operation, ambiguous: true },
    {
      operation,
      index_sql:
        "UPDATE articles SET authors_json='invalid' WHERE article_id=1012;",
    },
    { operation, index_sql: "DELETE FROM articles WHERE article_id=1012;" },
    {
      operation,
      index_sql:
        "UPDATE articles SET abstract_text=X'ff' WHERE article_id=1012;",
    },
  );
}
for (const references of [
  [
    { id: 1001, db: "metadata" },
    { id: 999, db: "absent" },
    { id: 1001, db: "metadata" },
  ],
  [{ id: 1012, db: "" }],
  [],
])
  requests.push({ operation: "citation", references });
requests.push(
  {
    operation: "citation",
    ambiguous: true,
    references: [{ id: 1001, db: "" }],
  },
  {
    operation: "citation",
    index_sql:
      "UPDATE articles SET authors_json='invalid' WHERE article_id=1012;",
    references: [
      { id: 1001, db: "metadata" },
      { id: 1012, db: "metadata" },
    ],
    error_category: true,
  },
  {
    operation: "citation",
    index_sql: "UPDATE articles SET abstract_text=X'ff' WHERE article_id=1012;",
    references: [{ id: 1012, db: "metadata" }],
  },
);
for (const id of [1001, 999, 0, -1])
  requests.push({ operation: "check", id, db: "metadata" });
for (const ids of [[], [1001, 1001, -1, 0, 1012, 999], Array(501).fill(1001)])
  requests.push({ operation: "batch", ids, db: "metadata" });
const result = spawnSync(
  "output/migration/execution/favorites-oracle.exe",
  [
    temporary,
    "tests/migration/storage/fixtures/metadata.sqlite.fixture",
    sqlPath,
  ],
  {
    cwd: WORKSPACE_ROOT,
    input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 32 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const cases = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(cases.length, requests.length);
const fixture = await fs.readFile(path.join(temporary, "auth.sqlite"));
await fs.writeFile(
  "tests/migration/storage/fixtures/favorites-auth.sqlite.fixture",
  fixture,
);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/business/favorites.rs",
  "crates/litradar-domain/src/business.rs",
  "crates/litradar-domain/src/validation.rs",
  "tests/migration/storage/favorites-oracle.rs",
  "tests/migration/storage/export-favorites.mjs",
  "tests/migration/storage/fixtures/metadata.sqlite.fixture",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/favorites-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, sources, fixture_sha256: digest(fixture), cases },
    null,
    2,
  ) + "\n",
);
console.log(`Exported ${cases.length} original favorite observations`);
