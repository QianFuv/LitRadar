/** Freeze original Rust identity, merge and catalog observations independently of Go. */
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
const article = {
  catalog_id: "catalog:example",
  title: "Example article",
  publication_year: 2026,
  date: "2026-08",
  issue_title: null,
  volume: "01",
  issue_number: "2",
  authors: [{ display_name: "Alice" }],
  start_page: "09",
  end_page: null,
  abstract_text: null,
  doi: "10.1000/example",
  pmid: "00042",
  open_access: null,
  in_press: null,
  retraction_dois: [],
};
const issue = {
  catalog_id: article.catalog_id,
  publication_year: 2026,
  title: "Issue title",
  volume: "01",
  number: "02",
  date: "2026-08",
};
const inputs = [];
for (const catalog_id of [
  "catalog:example",
  " CATALOG:EXAMPLE ",
  "İΣßＡ",
  "",
  "\u0000",
  "é",
  "e\u0301",
])
  inputs.push({ op: "journal", catalog_id });
for (const publication_year of [null, 0, -1, 2026])
  for (const volume of [null, "", "000", "Ｓ1", " A-2 "])
    for (const date of [null, "", " 2026-08 "])
      inputs.push({
        op: "issue",
        journal_id: "9007199254740993",
        issue: {
          ...issue,
          publication_year,
          volume,
          number: null,
          date,
          title: date === null ? "" : issue.title,
        },
      });
for (const doi of [
  null,
  "",
  "https://doi.org/10.1/A",
  "doi:10.X/İΣ",
  "10.1/a\u000b",
  "10.1/a\u000c",
  "bad",
])
  for (const pmid of [null, "", "000", "123", "１２"])
    for (const publication_year of [null, 2026])
      inputs.push({
        op: "identity",
        article: { ...article, doi, pmid, publication_year },
        aliases: [],
      });
for (const title of ["", "!!!", "  É A-B  ", "İΣͅ", "中文题名"])
  for (const volume of [null, "", "000", "1-A"])
    for (const date of [
      null,
      "2026-no-calendar",
      " 2026 -junk",
      "+2026",
      "999",
      "0000",
    ])
      inputs.push({
        op: "identity",
        article: {
          ...article,
          title,
          volume,
          date,
          publication_year: null,
          issue_number: null,
          start_page: null,
          doi: null,
          pmid: null,
        },
        aliases: [],
      });
for (const owners of [
  ["1", "1"],
  ["9007199254740993", "-7"],
  ["0", "0"],
])
  inputs.push({
    op: "identity",
    article,
    aliases: [
      { kind: "doi", value: article.doi, owner: owners[0] },
      { kind: "pmid", value: "42", owner: owners[1] },
    ],
  });
const variants = [
  article,
  { ...article, catalog_id: "other" },
  { ...article, doi: "10.1000/alternate" },
  { ...article, pmid: "43" },
  {
    ...article,
    title: "中文",
    publication_year: 2025,
    date: "2025-01-01",
    volume: "9",
    authors: [{ display_name: "Bob" }],
    abstract_text: "Short",
    in_press: true,
    open_access: false,
  },
  {
    ...article,
    title: "Longer example article",
    publication_year: 2027,
    date: "2027",
    volume: "10",
    authors: [{ display_name: "Á" }, { display_name: "A" }],
    abstract_text: "Longer detail",
    in_press: false,
    open_access: true,
  },
  {
    ...article,
    publication_year: null,
    date: null,
    volume: null,
    doi: null,
    pmid: null,
    authors: [],
    retraction_dois: ["10.2/b", "10.2/a", "10.2/a"],
  },
  {
    ...article,
    date: "not-a-date",
    publication_year: null,
    start_page: "99",
    issue_title: "New issue",
    retraction_dois: ["10.2/c", "10.2/a"],
  },
  { ...article, publication_year: 0, date: "0000-01", in_press: true },
];
for (const left of variants)
  for (const right of variants)
    for (const resolved of [false, true])
      inputs.push({ op: "merge", left, right, resolved });
const columns = [
  "catalog_id",
  "catalog_aliases",
  "title",
  "issn",
  "eissn",
  "all_issns",
  "title_aliases",
  "area",
  "utd_rank",
  "utd_rating",
  "abs_rank",
  "abs_rating",
  "fms_rank",
  "fms_rating",
  "fmscn_rank",
  "fmscn_rating",
];
const row = Object.fromEntries(columns.map((column) => [column, ""]));
Object.assign(row, {
  catalog_id: "journal-example",
  title: "Example Journal",
  issn: "1234-5679",
  all_issns: "1234-5679",
});
const csv = (values) => columns.map((column) => values[column]).join(",");
const header = columns.join(",");
for (const body of [
  "",
  "\n\t",
  header,
  header + "\n" + csv(row),
  header + "\r\n\n" + csv(row) + "\r\n",
  "\uFEFF" + header + "\n" + csv(row),
  header + '\n"bad',
  header + "\nshort",
  header + "\n" + csv({ ...row, title: '"A, B"' }),
  header + "\n" + csv({ ...row, title: 'A"B"C' }),
  header + "\n" + csv({ ...row, title: '"A""B"' }),
  header + "\n" + csv({ ...row, title: '"A\nB"' }),
])
  inputs.push({ op: "catalog", csv: body });
inputs.push({ op: "catalog_rows", rows: [] });
for (const field of columns) {
  const missing = { ...row };
  delete missing[field];
  inputs.push({ op: "catalog_rows", rows: [missing] });
  for (const value of ["", " ", "é", "a;a", " a ", "12345679;1234-5679"])
    inputs.push({ op: "catalog_rows", rows: [{ ...row, [field]: value }] });
}
inputs.push({ op: "catalog_rows", rows: [{ ...row, surprise: "" }] });
for (const name of [
  "\u0001",
  "\u0000",
  "\u0085",
  "\u200b",
  "\ue000",
  "\u0301",
  "\u00a0",
  "\n",
])
  inputs.push({ op: "catalog_rows", rows: [{ ...row, [name]: "" }] });
for (const second of [
  row,
  { ...row, catalog_id: "journal-other" },
  { ...row, catalog_id: "journal-other", catalog_aliases: row.catalog_id },
  {
    ...row,
    catalog_id: "journal-other",
    issn: "0378-5955",
    all_issns: "0378-5955",
  },
])
  inputs.push({ op: "catalog_rows", rows: [row, second] });
for (const path of [
  "metadata/.",
  "metadata/./.",
  "metadata/",
  "alpha.csv",
  ".csv",
  ".name.csv",
  "alpha.CSV",
  "alpha.csv/.",
  "alpha.csv/",
  "one/two/../alpha.csv",
  "α/期刊.csv",
  "./alpha.csv",
  "name.",
  "name..csv",
])
  inputs.push({ op: "catalog_path", path });
const result = spawnSync(build.binary, [], {
  input: inputs.map((input) => JSON.stringify(input)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, inputs.length);
await fs.writeFile(
  "tests/migration/index/identity-vectors.json",
  JSON.stringify(
    {
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-identity.mjs"),
      ),
      build,
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust indexing observations`,
);
