/** Freeze sequential client behavior against the original scholarly transport. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
/** Register a deterministic operation sequence using only synthetic upstream values. */
function add(id, fixture, operations, hasKey = false) {
  cases.push({ id, fixture, operations, has_semantic_scholar_key: hasKey });
}
/** Construct a source-page operation with explicit nullable bounds. */
function page(date = null, cursor = null) {
  return {
    op: "source_page",
    source_id: "https://openalex.org/S1",
    date,
    cursor,
  };
}
/** Construct a bounded Crossref request with an optional cursor. */
function crossref(cursor = null, from = -1, until = 1, issn = "1234-5678") {
  return {
    op: "crossref",
    issn,
    query: { kind: "works", created_from: from, created_until: until, cursor },
  };
}
for (const restriction of [false, true]) {
  for (const status of [200, 201, 400, 403, 429, 500]) {
    add(
      `fallback-${restriction}-${status}`,
      {
        openalex_source_works_plan_restricted: restriction,
        openalex_source_works_status: status,
        openalex_source_work_pages: [[{ id: "W1" }], [], [{ id: "W3" }]],
      },
      [
        page("2026-01-01", "fixture-page-1"),
        { op: "drain" },
        page("2026-01-01", "fixture-page-2"),
        page(null, "fixture-page-2"),
        page("", "fixture-page-1"),
      ],
    );
  }
}
for (const cursor of [
  null,
  "*",
  "fixture-page-0",
  "fixture-page-+1",
  "fixture-page-1",
  "fixture-page-2",
  "fixture-page-18446744073709551614",
  "fixture-page--1",
  "fixture-page- 1",
  "fixture-page-1 ",
  "fixture-page-++1",
]) {
  add(
    `cursor-${cursor}`,
    {
      openalex_source_work_pages: [[1], [], [3]],
      openalex_source_works_plan_restricted_after_page: 1,
    },
    [page(null, cursor), page("date", cursor)],
  );
}
for (const count of [0, 1, 224, 225, 226, 450, 451]) {
  const works = Array.from({ length: count }, (_, index) => ({
    DOI: `10.1/${index}`,
    created: { timestamp: 0 },
  }));
  add(
    `crossref-${count}`,
    {
      crossref_work_pages: [works.slice(0, 3), works.slice(3)],
      crossref_works: [{ ignored: true }],
    },
    [
      crossref("*"),
      crossref("same"),
      crossref("same"),
      { op: "drain" },
      crossref(null),
      {
        op: "crossref",
        issn: "OTHER",
        query: { kind: "earliest_created", until: 0 },
      },
      crossref("continuation", -1, 1, "OTHER"),
    ],
  );
}
add(
  "crossref-created-types",
  {
    crossref_works: [
      { id: 1 },
      { created: null },
      { created: { timestamp: -1 } },
      { created: { timestamp: -1001 } },
      { created: { timestamp: "bad" } },
      true,
      null,
      { created: { timestamp: 2000 } },
    ],
  },
  [
    crossref("*", -2, -1),
    crossref(null, 0, 0),
    {
      op: "crossref",
      issn: "X",
      query: { kind: "earliest_created", until: 5 },
    },
  ],
);
for (const status of [201, 400, 429, 500])
  add(`crossref-status-${status}`, { crossref_status: status }, [
    crossref(),
    { op: "drain" },
    crossref("*"),
  ]);
for (const source of [
  null,
  {},
  { issn_l: "1234-567X" },
  { issn: [null, 5, "1234567x"] },
  { display_name: "  JOURNAL\tOF ΟΣ " },
  { display_name: "Journal of oS extra" },
]) {
  add(
    `source-${JSON.stringify(source)}`,
    { openalex_source_by_issns: source, openalex_source_by_title: source },
    [
      { op: "issns", issns: ["bad", "bad", "1234-567x", "other"] },
      { op: "title", title: " \t\n " },
      { op: "title", title: "journal of ος" },
    ],
  );
}
const doiValues = [
  null,
  "",
  " DOI: 10.1/ABC ",
  "HTTPs://DOI.ORG/strange",
  true,
  [],
  { Z: [1.5], a: "<>&\u2028" },
  0,
  -0.0,
  0.000001,
  10000000000000000,
];
add(
  "normalize-values",
  {},
  doiValues.map((value) => ({ op: "normalize", value })),
);
for (const hasKey of [false, true])
  for (const batchSize of [0, 1, 2, 100, 500, 900]) {
    add(
      `doi-batches-${hasKey}-${batchSize}`,
      {
        openalex_by_doi: {
          "10.1/a": { doi: "DOI:10.1/REPLACED", title: "first" },
          "10.1/b": { doi: "10.1/replaced", title: "last" },
          "10.1/c": { doi: true },
        },
        semantic_scholar_by_doi: {
          "10.1/a": { externalIds: { DOI: "10.1/replaced" }, title: "first" },
          "10.1/b": { externalIds: { DOI: "10.1/replaced" }, title: "last" },
          "10.1/c": { externalIds: { DOI: true } },
        },
      },
      [
        { op: "s2_dois", dois: ["", " "], batch_size: batchSize },
        { op: "openalex_dois", dois: [], batch_size: batchSize },
        ...["openalex_dois", "s2_dois"].map((op) => ({
          op,
          dois: ["DOI:10.1/A", "10.1/a", " 10.1/b ", "10.1/c", "missing"],
          batch_size: batchSize,
        })),
      ],
      hasKey,
    );
  }
for (const status of [200, 201, 400, 403, 429, 500])
  for (const error of [
    " no valid PAPER ids given ",
    "no valid paper ids given!",
    "secret error",
  ]) {
    add(
      `s2-error-${status}-${error}`,
      { semantic_scholar_status: status, semantic_scholar_error: error },
      [{ op: "s2_dois", dois: ["a", "b", "c"], batch_size: 1 }],
      true,
    );
  }
for (const length of [100, 1600, 1750, 1800, 1900])
  add(`doi-url-budget-${length}`, {}, [
    {
      op: "openalex_dois",
      dois: ["10.1/" + "x".repeat(length), "10.1/汉字*~"],
      batch_size: 100,
    },
  ]);

const result = spawnSync(
  "output/migration/execution/sources-scholarly-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.equal(result.status, 0, result.stderr || String(result.error));
assert.ifError(result.error);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/scholarly-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-scholarly-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-scholarly.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust scholarly operation sequences`,
);
