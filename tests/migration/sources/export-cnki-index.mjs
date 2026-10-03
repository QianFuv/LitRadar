/** Freeze CNKI canonical conversion and opaque-state acceptance against original Rust. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const cases = [];
/** Preserve raw state representations and exact integer tokens. */
function add(kind, input) {
  cases.push({
    id: `${kind}-${cases.length}`,
    kind,
    input: typeof input === "string" ? input : JSON.stringify(input),
  });
}
const catalog = {
  catalog_id: "cnki",
  catalog_aliases: [],
  title: "Journal",
  issn: null,
  eissn: null,
  all_issns: [],
  title_aliases: [],
  area: null,
  rankings: {},
};
const issue = { year: 2026, number: "01", volume: "2", title: "Issue" };
const summary = {
  title: "Summary",
  authors: "张三",
  date: "2026-02",
  pages: "1-9",
};
const detail = {
  title: "Title",
  authors: "李四",
  doi: "10.1234/test",
  pmid: "123",
  open_access: true,
};
for (const authors of [
  "",
  " ",
  "abc",
  "123;①†",
  "1.张三2a;李四①;王五b",
  "研究小组1;北京大学2",
  "Émile, First Last",
  "甲乙丙丁戊己庚辛壬9",
  "中·文*；a；Alpha",
  null,
  12,
  false,
  [],
])
  add("cnki_article", {
    catalog,
    issue,
    summary,
    detail: { ...detail, authors, doi: null },
  });
for (const field of [
  "title",
  "authors",
  "pages",
  "online_release_date",
  "date",
  "publication_date",
  "doi",
  "pmid",
  "open_access",
  "retraction_doi",
  "abstract",
])
  for (const value of [
    undefined,
    null,
    "",
    "  ",
    "invalid",
    "2025-01-02",
    "TRUE",
    "no",
    true,
    false,
    0,
    -1,
    1.5,
    [],
    {},
  ])
    add("cnki_article", {
      catalog,
      issue,
      summary,
      detail: { ...detail, [field]: value },
    });
for (const year of [-123, -1, 0, 99, 2026, "+2025", "2025.0", "-1", null, 1.5])
  for (const number of [null, "0", "01", "+2", "13", "-1", "256", " 3 "])
    add("cnki_article", {
      catalog,
      issue: { ...issue, year, number },
      summary: {},
      detail,
    });
for (const integer of [
  "9223372036854775807",
  "-9223372036854775808",
  "18446744073709551615",
])
  add(
    "cnki_article",
    JSON.stringify({
      catalog,
      issue: { ...issue, year: "INTEGER" },
      summary: {},
      detail,
    }).replace('"INTEGER"', integer),
  );
for (const kind of ["anchor", "checkpoint"]) {
  const state =
    kind === "anchor"
      ? { version: 1, year_issue_id: "202601" }
      : {
          version: 2,
          base_anchor_issue_id: null,
          candidate_head_issue_id: "202601",
          current_issue_id: "202601",
          page_index: 0,
        };
  add(`cnki_${kind}`, state);
  add(`cnki_${kind}`, Object.values(state));
  for (const field of Object.keys(state))
    for (const value of [undefined, null, "", 0, 3, {}, []])
      add(`cnki_${kind}`, { ...state, [field]: value });
  for (const forbidden of [
    "captcha",
    "SeCrEtKeY",
    "pointjson",
    "jfbym",
    "session",
    "cookie",
    "token",
    "http://",
    "https://",
    "url",
    "curly",
    "safe",
    "t\\u006fken",
  ])
    add(
      `cnki_${kind}`,
      JSON.stringify(state).replace('"202601"', `"${forbidden}"`),
    );
}
const result = spawnSync(
  "output/migration/execution/sources-index-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout.trim().split(/\r?\n/);
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/cnki-index-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-index-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-cnki-index.mjs"),
      ),
    },
    null,
    2,
  ).replace(
    /\n}$/,
    `,\n  "observations": [\n${observations.join(",\n")}\n]\n}\n`,
  ),
);
console.log(
  `Frozen ${observations.length} original Rust CNKI index observations`,
);
