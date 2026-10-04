/** Freeze complete content transactions and their canonical SQLite projections using original Rust. */
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
const catalog = {
  catalog_id: "journal-example",
  catalog_aliases: [],
  title: "Example Journal",
  title_aliases: [],
  issn: "1234-5679",
  eissn: null,
  all_issns: ["1234-5679"],
  area: null,
  rankings: {
    utd_rank: null,
    utd_rating: null,
    abs_rank: null,
    abs_rating: null,
    fms_rank: null,
    fms_rating: null,
    fmscn_rank: null,
    fmscn_rating: null,
  },
};
const article = {
  catalog_id: catalog.catalog_id,
  title: "Example article",
  publication_year: 2026,
  date: "2026-08",
  issue_title: null,
  volume: "1",
  issue_number: "2",
  authors: [{ display_name: "Alice" }],
  start_page: "9",
  end_page: null,
  abstract_text: null,
  doi: "10.1000/example",
  pmid: null,
  open_access: null,
  in_press: null,
  retraction_dois: [],
};
/** Construct one complete provider-neutral write with a stable revision. */
function write(
  articles = [article],
  entry = catalog,
  revision = "revision:1",
  issues = [],
) {
  return {
    op: "write",
    catalog: entry,
    batch: {
      catalog_id: entry.catalog_id,
      journal: {
        catalog_id: entry.catalog_id,
        observed_title: null,
        observed_issns: [],
        observed_title_aliases: [],
      },
      issues,
      articles,
      progress: { state: "complete", next_anchor: null },
    },
    revision,
  };
}
const requests = [];
/** Preserve the operation order for a single independent in-memory database. */
function scenario(name, operations) {
  requests.push({ op: "content", name, operations });
}
scenario("empty", [write([])]);
scenario("first-and-replay", [
  write(),
  write(),
  write([article], catalog, "another-revision"),
]);
scenario("multiple-articles", [
  write([
    article,
    { ...article, title: "Another", start_page: "10", doi: "10.1000/second" },
  ]),
]);
for (const [field, value] of Object.entries({
  title: "A longer article title",
  publication_year: 2027,
  date: "2026-08-15",
  issue_title: "Named issue",
  volume: "3",
  issue_number: "4",
  authors: [{ display_name: "Alice" }, { display_name: "Bob" }],
  start_page: "7",
  end_page: "20",
  abstract_text: "Nonempty <text> & \u2028 中文",
  doi: "10.1000/alternate",
  pmid: "9007199254740993",
  open_access: true,
  in_press: false,
  retraction_dois: ["10.1000/retract-a", "10.1000/retract-b"],
})) {
  const changed = { ...article, [field]: value };
  if (field === "publication_year") changed.date = "2027-01";
  scenario(`change-${field}`, [
    write(),
    write([changed], catalog, "changed"),
    write([changed], catalog, "replayed"),
  ]);
}
scenario("retraction-union", [
  write([{ ...article, retraction_dois: ["10.1/a", "10.1/b"] }]),
  write(
    [{ ...article, retraction_dois: ["10.1/b", "10.1/c"] }],
    catalog,
    "union",
  ),
]);
scenario("alternate-dois-in-page", [
  write([article, { ...article, doi: "10.1000/alternate" }]),
  write([{ ...article, doi: "10.1000/alternate" }, article]),
]);
scenario("formal-not-regressed", [
  write([{ ...article, in_press: false }]),
  write(
    [{ ...article, in_press: true, date: "2025-01", publication_year: 2025 }],
    catalog,
    "later",
  ),
]);
scenario("move-shared-issue", [
  write([
    article,
    { ...article, title: "Second", start_page: "10", doi: "10.1000/second" },
  ]),
  write(
    [{ ...article, date: "2027-01", publication_year: 2027 }],
    catalog,
    "move-first",
  ),
  write(
    [
      {
        ...article,
        title: "Second",
        start_page: "10",
        doi: "10.1000/second",
        date: "2027-01",
        publication_year: 2027,
      },
    ],
    catalog,
    "move-second",
  ),
]);
scenario("date-refresh", [
  write([
    { ...article, date: "2026-08-07" },
    {
      ...article,
      title: "Second",
      start_page: "10",
      doi: "10.1000/second",
      date: "2026-08-08",
    },
  ]),
  write([{ ...article, date: "2026-08-10" }], catalog, "correction"),
]);
for (const field of ["area", "title"])
  scenario(`journal-${field}`, [
    write(),
    write([], { ...catalog, [field]: "Changed" }, "metadata"),
    write([], { ...catalog, [field]: "Changed" }, "metadata-replay"),
  ]);
scenario("reconcile-metadata", [
  write(),
  {
    op: "reconcile",
    catalogs: [{ ...catalog, title: "Changed", area: "New area" }],
  },
]);
scenario("alias-bridge-rolls-back-page", [
  write([
    article,
    { ...article, title: "Other", start_page: "22", doi: "10.1000/other" },
  ]),
  write(
    [
      { ...article, title: "New", start_page: "33", doi: "10.1000/new" },
      { ...article, doi: "10.1000/other" },
    ],
    catalog,
    "conflict",
  ),
]);
scenario("invalid-revision", [write([article], catalog, " ")]);
scenario("empty-title-doi", [
  write([
    {
      ...article,
      title: "",
      publication_year: null,
      date: null,
      volume: null,
      issue_number: null,
      start_page: null,
    },
  ]),
]);
scenario("invalid-batch-before-revision", [
  write([{ ...article, title: "", doi: null }], catalog, " "),
]);
scenario("legacy-empty-shell", [
  write([], {
    ...catalog,
    catalog_id: "journal-old",
    all_issns: [],
    issn: null,
  }),
  {
    op: "reconcile",
    catalogs: [{ ...catalog, catalog_aliases: ["journal-old"] }],
  },
  write([article], { ...catalog, catalog_aliases: ["journal-old"] }),
]);
scenario("legacy-shell-history-rejected", [
  write([{ ...article, catalog_id: "journal-old" }], {
    ...catalog,
    catalog_id: "journal-old",
  }),
  {
    op: "reconcile",
    catalogs: [{ ...catalog, catalog_aliases: ["journal-old"] }],
  },
]);
scenario("reconcile-conflict-rolls-back-shell-cleanup", [
  write([], {
    ...catalog,
    catalog_id: "journal-old",
    all_issns: [],
    issn: null,
  }),
  write([], { ...catalog, catalog_id: "journal-owner" }),
  {
    op: "reconcile",
    catalogs: [{ ...catalog, catalog_aliases: ["journal-old"] }],
  },
]);
scenario("existing-alias-required-for-hash", [
  write(),
  { op: "sql", sql: "DELETE FROM article_identity_keys" },
  write(),
]);
for (const table of [
  "articles",
  "article_listing",
  "article_search",
  "article_identity_keys",
  "article_change_events",
]) {
  if (table === "article_search") continue;
  scenario(`late-fault-${table}`, [
    {
      op: "sql",
      sql: `CREATE TRIGGER injected BEFORE INSERT ON ${table} BEGIN SELECT RAISE(ABORT,'injected'); END;`,
    },
    write(),
    { op: "sql", sql: "DROP TRIGGER injected" },
  ]);
}
scenario("issue-title-scalar-length", [
  write([], catalog, "issue-one", [
    {
      catalog_id: catalog.catalog_id,
      publication_year: 2026,
      title: "界界",
      volume: "1",
      number: "2",
      date: "2026-08",
    },
  ]),
  write([], catalog, "issue-two", [
    {
      catalog_id: catalog.catalog_id,
      publication_year: 2026,
      title: "abc",
      volume: "1",
      number: "2",
      date: "2026-08",
    },
  ]),
]);
for (const [ordinal, value] of [
  "null",
  "[null]",
  "[{}]",
  '[{"display_name":null}]',
  '[{"x":0}]',
  "[{},",
  "[]",
  '[["Alice"]]',
  '[{"display_name":""}]',
  "{}",
  "true",
  "42",
  '"name"',
  "[true]",
  "[42]",
  '["name"]',
  '[{"display_name":1}]',
  '[{"display_name":true}]',
  '[{"display_name":{}}]',
  '[{"display_name":[]}]',
  '[{"display_name":"a","display_name":"b"}]',
  '[{"display_name":"a","x":0}]',
  "[[]]",
  '[["a","b"]]',
  '[{"display_name":"Alice"}]',
  "",
  "[",
  "[{",
  '[{"display_name":',
  '[{"display_name":"a"}',
  '[["a"]',
  "[] null",
  '[["a",]]',
  '[["a"],',
  "-0",
  "1e1",
  "1e999",
  "18446744073709551616",
  "1e-7",
  '"\\q"',
  '"abc',
  '"\\u0000"',
  '"\\u0301"',
  '[{"display_name":"\\ud800"}]',
  '"\\udc00"',
  '"\\u12"',
  '[\n {"display_name":null}\n]',
].entries()) {
  scenario(`stored-authors-${ordinal}`, [
    write(),
    {
      op: "sql",
      sql: `UPDATE articles SET authors_json='${value.replaceAll("'", "''")}'`,
    },
    write([article], catalog, "after-corruption"),
  ]);
}
const result = spawnSync(build.binary, [], {
  input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.equal(result.status, 0, result.stderr);
assert.ifError(result.error);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, requests.length);
const replay = observations.find(
  (value) => value.input.name === "first-and-replay",
).expected;
assert.equal(
  replay.operations[0].articles_changed,
  1,
  "The original writer must actually insert the fixture",
);
assert.equal(
  replay.operations[1].articles_changed,
  0,
  "The second write must exercise replay",
);
assert.equal(replay.tables.articles.length, 1);
assert.equal(replay.tables.article_change_events.length, 1);
assert.equal(
  observations.find((value) => value.input.name === "move-shared-issue")
    .expected.tables.articles.length,
  2,
);
await fs.writeFile(
  "tests/migration/index/content-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-content.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust content workflows`);
