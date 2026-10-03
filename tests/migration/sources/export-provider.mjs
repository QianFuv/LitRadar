/** Capture original canonical contract normalization, validation and serialization. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const requests = [];
const rankings = {
  utd_rank: null,
  utd_rating: null,
  abs_rank: null,
  abs_rating: null,
  fms_rank: null,
  fms_rating: null,
  fmscn_rank: null,
  fmscn_rating: null,
};
const catalog = {
  catalog_id: "journal.test",
  catalog_aliases: [],
  title: "Journal of Testing",
  issn: "0378-5955",
  eissn: null,
  all_issns: ["0378-5955"],
  title_aliases: ["Testing Journal"],
  area: null,
  rankings,
};
const journal = {
  catalog_id: catalog.catalog_id,
  observed_title: null,
  observed_issns: [],
  observed_title_aliases: [],
};
const issue = {
  catalog_id: catalog.catalog_id,
  publication_year: 2026,
  title: null,
  volume: "1",
  number: null,
  date: "2026-02",
};
const article = {
  catalog_id: catalog.catalog_id,
  title: "Article",
  publication_year: null,
  date: null,
  issue_title: null,
  volume: null,
  issue_number: null,
  authors: [],
  start_page: null,
  end_page: null,
  abstract_text: null,
  doi: "10.1/test",
  pmid: null,
  open_access: null,
  in_press: null,
  retraction_dois: [],
};
const batch = {
  catalog_id: catalog.catalog_id,
  journal,
  issues: [],
  articles: [article],
  progress: { state: "complete" },
};
for (const input of [
  "",
  " ",
  "\u00a0\u2003",
  "Café",
  "Cafe\u0301",
  "İSTANBUL",
  "ΑΓΟΣ",
  "ΟΣ ΑΣ",
  "ὈΔΥΣΣΕΎΣ",
  "A\u0345B",
  "123",
  "000",
  "...",
  "A--B_C",
  "2026",
  "0000",
  "0999",
  "2026-02",
  "2024-02-29",
  "2026-02-29",
  "2026-02-31",
  "2026-2",
  "2026-00",
  "2026-01-00",
  "0378-5955",
  "03785955",
  " 0378 - 5955 ",
  "1234-567X",
  "1234-5679",
  "https://doi.org/10.A/ABC",
  "doi: 10.A/İΟΣ",
  "10.x/a\vb",
  "10.x/a\tb",
  "10.x/a\u00a0b",
  "10.x/http://x",
  "000123",
  "００１",
  "ß",
  "ﬃ",
  "中 文",
  "a\u200db",
])
  requests.push({ kind: "normalize", input });
for (let point = 0; point < 0x400; point += 3)
  requests.push({
    kind: "normalize",
    input: String.fromCodePoint(point) + "AΣ",
  });
requests.push({ kind: "catalog", input: catalog });
for (const [key, value] of [
  ["catalog_id", "bad ID"],
  ["catalog_aliases", [catalog.catalog_id]],
  ["catalog_aliases", ["ok-id", "ok-id"]],
  ["title", " Cafe\u0301 "],
  ["issn", "03785955"],
  ["all_issns", []],
  ["all_issns", ["0378-5955", "0378-5955"]],
  ["title_aliases", ["journal-of-testing"]],
  ["title_aliases", ["Alias", "ALIAS"]],
  ["area", " "],
])
  requests.push({ kind: "catalog", input: { ...catalog, [key]: value } });
for (const name of Object.keys(rankings))
  requests.push({
    kind: "catalog",
    input: { ...catalog, rankings: { ...rankings, [name]: " untrimmed " } },
  });
requests.push({ kind: "batch", input: { catalog, batch } });
for (const [key, value] of [
  ["catalog_id", "other"],
  ["title", ""],
  ["title", "Article "],
  ["publication_year", 999],
  ["publication_year", 10000],
  ["date", "2026-02-31"],
  ["date", "2026-13-01"],
  ["date", "2026-01-32"],
  ["date", "2026-1"],
  ["date", "0000"],
  ["doi", "DOI:10.1/test"],
  ["doi", null],
  ["pmid", "001"],
  ["authors", [{ display_name: " untrimmed " }]],
  ["retraction_dois", ["10.1/b", "10.1/a"]],
  ["retraction_dois", ["10.1/a", "10.1/a"]],
])
  requests.push({
    kind: "batch",
    input: {
      catalog,
      batch: { ...batch, articles: [{ ...article, [key]: value }] },
    },
  });
for (const [key, value] of [
  ["observed_title", "Other"],
  ["observed_title", "Testing Journal"],
  ["observed_issns", ["1234-5679"]],
  ["observed_issns", ["0378-5955", "0378-5955"]],
  ["observed_title_aliases", ["Other"]],
])
  requests.push({
    kind: "batch",
    input: {
      catalog,
      batch: { ...batch, journal: { ...journal, [key]: value } },
    },
  });
for (const change of [
  {},
  { date: "2026-02-31" },
  { publication_year: 2025 },
  { publication_year: null, volume: null, date: null },
  { publication_year: null, volume: null, date: null, title: "Issue" },
  { volume: " " },
])
  requests.push({
    kind: "batch",
    input: { catalog, batch: { ...batch, issues: [{ ...issue, ...change }] } },
  });
for (const progress of [
  { state: "continue", checkpoint: "" },
  { state: "continue", checkpoint: " " },
  { state: "continue", checkpoint: "界".repeat(21846) },
  { state: "complete", next_anchor: "" },
  { state: "complete", next_anchor: null },
])
  requests.push({
    kind: "batch",
    input: { catalog, batch: { ...batch, progress } },
  });
for (const input of [
  "https://doi.org/10.x/a",
  "http://example.org",
  "HTTPS://example.org",
  "https://",
  "https://user@example.org",
  "https://example.org\n",
  "https://example .org",
  "https://example.org/path?q=a#b",
  "https://example.org/" + "x".repeat(8192),
])
  requests.push({ kind: "redirect", input });
for (const change of [
  {},
  { size: 0 },
  { maximum: 0 },
  { size: 65 },
  { content_type: "Application/pdf" },
  { content_type: "application/pdf; charset=utf8" },
  { content_type: "text/plain" },
  { filename: "../x.pdf" },
  { filename: "x\\y.pdf" },
  { filename: "界".repeat(86) },
  { filename: "file.pdf" },
])
  requests.push({
    kind: "document",
    input: {
      content_type: "application/pdf",
      filename: null,
      size: 64,
      maximum: 64,
      ...change,
    },
  });
for (const name of [
  "ok",
  "a",
  "a".repeat(64),
  "a".repeat(65),
  "1p",
  "_p",
  "-p",
  "p_",
  "p-p",
  "A-name",
  "中",
])
  requests.push({
    kind: "registry",
    input: {
      name,
      capabilities: [true, false, false],
      implementations: [true, false, false],
      hosts: [],
    },
  });
for (const hosts of [
  [],
  ["doi.org"],
  ["DOI.org"],
  ["localhost"],
  ["127.0.0.1"],
  ["doi.org."],
  ["-doi.org"],
  ["doi-.org"],
  ["doi..org"],
  ["https://doi.org"],
  ["doi.org:443"],
  ["doi.org", "doi.org"],
  ["a".repeat(64) + ".org"],
])
  requests.push({
    kind: "registry",
    input: {
      name: "sample",
      capabilities: [false, true, false],
      implementations: [false, true, false],
      hosts,
    },
  });
for (const capabilities of [
  [false, false, false],
  [true, false, false],
  [false, true, false],
  [false, false, true],
  [true, true, true],
])
  for (const implementations of [
    [false, false, false],
    [true, false, false],
    [false, true, false],
    [true, true, true],
  ])
    requests.push({
      kind: "registry",
      input: {
        name: "sample",
        capabilities,
        implementations,
        hosts: ["doi.org"],
      },
    });
const typed = {
  catalog,
  rankings,
  journal,
  issue,
  author: { display_name: "Author" },
  article,
  batch,
  progress: { state: "complete" },
};
for (const [type, value] of Object.entries(typed)) {
  const serialized = JSON.stringify(value);
  for (const input of [
    serialized,
    "{}",
    "null",
    "[]",
    JSON.stringify(Object.values(value)),
    serialized.replace("{", '{"unknown":1,'),
    serialized.replace("{", `{${JSON.stringify(Object.keys(value)[0])}:null,`),
  ])
    requests.push({ kind: "decode", type, input });
  for (const key of Object.keys(value)) {
    const missing = { ...value };
    delete missing[key];
    requests.push({ kind: "decode", type, input: JSON.stringify(missing) });
    requests.push({
      kind: "decode",
      type,
      input: JSON.stringify({ ...value, [key]: null }),
    });
  }
}
for (const input of [
  '{"state":"complete","checkpoint":null}',
  '{"state":"continue","checkpoint":"x","next_anchor":null}',
  '{"state":"continue","checkpoint":null}',
  '{"state":"other"}',
  '{"state":"complete","state":"complete"}',
  '["complete"]',
  '["complete",null]',
  '["complete","x"]',
  '["complete",null,null]',
  '["continue"]',
  '["continue","x"]',
  '["continue",null]',
  '["continue",{"checkpoint":"x"}]',
  '["continue","x","extra"]',
])
  requests.push({ kind: "decode", type: "progress", input });
for (const input of [
  '"bootstrap"',
  '"incremental"',
  '"full_rescan"',
  '"other"',
  "null",
  '{"bootstrap":null}',
  '{"bootstrap":[]}',
  '{"bootstrap":{}}',
  '{"bootstrap":null,"full_rescan":null}',
  '["bootstrap"]',
])
  requests.push({ kind: "decode", type: "mode", input });
for (const year of [
  "-0",
  "2026.0",
  "2.026e3",
  "9223372036854775808",
  "-9223372036854775808",
])
  requests.push({
    kind: "decode",
    type: "issue",
    input: JSON.stringify(issue).replace(
      '"publication_year":2026',
      `"publication_year":${year}`,
    ),
  });
requests.push(
  { kind: "decode", type: "author", input: '{"display_name":"\\ud800"}' },
  {
    kind: "decode",
    type: "article",
    input: JSON.stringify({ ...article, authors: [null] }),
  },
);
const result = spawnSync(
  "output/migration/execution/sources-provider-oracle.exe",
  [],
  {
    input: requests.map((item) => JSON.stringify(item)).join("\n") + "\n",
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
assert.equal(observations.length, requests.length);
await fs.writeFile(
  "tests/migration/sources/provider-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-provider-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-provider.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust provider observations`,
);
