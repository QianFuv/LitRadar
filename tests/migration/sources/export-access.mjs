/** Freeze public abstract adapters with exact identifier escaping and fixture-driven CNKI lookup. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const cases = [];
/** Register one independent provider invocation. */
function add(kind, article, fixture) {
  cases.push({
    id: `${kind}-${cases.length}`,
    kind,
    article,
    ...(fixture ? { fixture } : {}),
  });
}
for (const doi of [
  null,
  "",
  " ",
  "10.1234/test",
  "10.1234/a b?#%",
  "10.1234/中文/𝄞",
  "doi:10.1234/ABC",
  "a\u0000b",
  "A-._~/+&=",
])
  for (const pmid of [null, "", "12345"]) add("scholarly", { doi, pmid });
const corpus = JSON.parse(
  await fs.readFile("tests/migration/sources/cnki-vectors.json", "utf8"),
);
const data = corpus.observations.find(
  (value) =>
    value.kind === "fixture" &&
    value.fixture?.journal_detail_html &&
    !value.fixture.fail_endpoint,
).fixture;
assert.ok(data);
data.article_detail_html.SJJJ202512002 = data.article_detail_html.id;
data.issue_article_pages["202511"] = ['<input id="articleCount" value="0">'];
const base = {
  title: "建立互利共赢的标准化合作伙伴关系",
  journal_title: "世界经济",
  journal_issns: ["1002-9621"],
  publication_year: 2025,
  issue_number: "12",
};
for (const title of [base.title, "贸易冲击：研究", "missing", "", "　"])
  add("cnki", { ...base, title }, data);
for (const journal_title of ["世界经济", "missing", "", "　"])
  for (const journal_issns of [[], [" "], ["1002-9621"], ["bad"]])
    add("cnki", { ...base, journal_title, journal_issns }, data);
for (const publication_year of [null, 2025, 2024])
  for (const issue_number of [null, "12", "1"])
    add("cnki", { ...base, publication_year, issue_number }, data);
for (const fail_endpoint of [
  "journal_detail",
  "year_issues",
  "issue_articles",
  "article_detail",
])
  add("cnki", base, { ...data, fail_endpoint });
for (const status of [400, 404, 410, 429, 500])
  add("cnki", base, {
    ...data,
    article_detail_status_codes: { SJJJ202512002: status },
  });
for (const doi of [null, "10.1000/domestic.sample", "10.1000/other"])
  add("cnki", { ...base, doi }, data);
for (const title of ["wrong title", base.title])
  add("cnki", base, {
    ...data,
    article_detail_html: {
      SJJJ202512002: `<html><head><title>${title} - 中国知网</title></head><body><span>DOI：</span><p>10.1000/domestic.sample</p></body></html>`,
    },
  });
const result = spawnSync(
  "output/migration/execution/sources-access-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
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
await fs.writeFile(
  "tests/migration/sources/access-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-access-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-access.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust access observations`);
