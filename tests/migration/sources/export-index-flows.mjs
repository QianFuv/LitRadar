/** Freeze complete canonical index provider traversals including cache-ahead replay and fallback. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const catalog = {
  catalog_id: "journal",
  catalog_aliases: [],
  title: "Journal",
  issn: "1234-5679",
  eissn: null,
  all_issns: ["1234-5679"],
  title_aliases: [],
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
const cases = [];
/** Record one bounded isolated source traversal. */
function add(id, fixture, options = {}) {
  cases.push({
    id,
    kind: "workflow",
    input: JSON.stringify({ catalog, mode: "bootstrap", fixture, ...options }),
  });
}
/** Create a complete Crossref record whose creation and publication dates have distinct roles. */
function work(index, issue = 1) {
  return {
    DOI: `10.1234/${index}`,
    title: [`Work ${index}`],
    created: { timestamp: 1000000 },
    published: { "date-parts": [[2026, issue, 1]] },
    volume: "1",
    issue: String(issue),
    abstract: "<p>Abstract</p>",
    author: [{ given: "A", family: "B" }],
  };
}
/** Freeze an incremental base issue in the existing v1 anchor representation. */
function anchor(issue) {
  return JSON.stringify({
    version: 1,
    issue: {
      kind: "volume_issue",
      publication_year: 2026,
      volume: "1",
      issue: String(issue),
    },
    from_sync_date: "2026-01-01",
  });
}
const source = {
  id: "https://openalex.org/S1",
  display_name: "Journal",
  issn: ["1234-5679"],
};
const oa = {
  doi: "https://doi.org/10.1234/oa",
  display_name: "OA work",
  publication_date: "2026-02-01",
  publication_year: 2026,
  biblio: { volume: "1", issue: "2" },
};
for (const count of [0, 1, 2, 225, 226, 450])
  add(`crossref-${count}`, {
    crossref_works: Array.from({ length: count }, (_, index) => work(index)),
  });
for (const step of [1, 2])
  add(`lost-ack-${step}`, { crossref_works: [work(0)] }, { replay_at: [step] });
for (const status of [404, 400, 429, 500])
  add(`crossref-status-${status}`, {
    crossref_status: status,
    openalex_source_by_issns: source,
    openalex_source_works: [oa],
  });
for (const key of [false, true])
  for (const semanticStatus of [200, 400, 500])
    add(
      `enrichment-${key}-${semanticStatus}`,
      {
        crossref_works: [work(0)],
        semantic_scholar_status: semanticStatus,
        semantic_scholar_by_doi: {
          "10.1234/0": {
            externalIds: { DOI: "10.1234/0" },
            isOpenAccess: false,
          },
        },
        openalex_by_doi: {
          "10.1234/0": {
            doi: "https://doi.org/10.1234/0",
            best_oa_location: {},
          },
        },
      },
      { has_key: key },
    );
for (const issue of [1, 2, 3, 4])
  add(
    `incremental-base-${issue}`,
    { crossref_works: [work(0, 3), work(1, 2), work(2, 1)] },
    { mode: "incremental", anchor: anchor(issue) },
  );
add(
  "unknown-issue",
  {
    crossref_works: [
      {
        DOI: "10.1234/unknown",
        title: ["Unknown"],
        created: { timestamp: 1000000 },
      },
      work(1, 2),
    ],
  },
  { mode: "incremental", anchor: anchor(2) },
);
for (const restriction of [false, true])
  add(
    `oa-filter-${restriction}`,
    {
      openalex_source_by_title: source,
      openalex_source_work_pages: [
        [oa],
        [
          {
            ...oa,
            doi: "10.1234/old",
            publication_date: "2026-01-01",
            biblio: { volume: "1", issue: "1" },
          },
        ],
        [],
      ],
      openalex_source_works_plan_restricted: restriction,
    },
    {
      catalog: { ...catalog, issn: null, all_issns: [] },
      mode: "incremental",
      anchor: anchor(1),
    },
  );
add(
  "invalid-anchor",
  { crossref_works: [work(0)] },
  { mode: "incremental", anchor: "bad" },
);
add("missing-title-identified", {
  crossref_works: [
    {
      DOI: "10.1234/no-title",
      created: { timestamp: 1000000 },
      published: { "date-parts": [[2026, 1, 1]] },
    },
  ],
});
add("unrepresentable-selected", {
  crossref_works: [
    {
      title: ["No Identity"],
      created: { timestamp: 1000000 },
      published: { "date-parts": [[2026, 1, 1]] },
    },
  ],
});
add(
  "alternate-issns",
  {
    crossref_status: 404,
    openalex_source_by_issns: source,
    openalex_source_works: [oa],
  },
  { catalog: { ...catalog, eissn: "2049-3630" } },
);
add(
  "old-crossref",
  { crossref_works: [work(0)] },
  {
    mode: "incremental",
    anchor: anchor(1),
    checkpoint: JSON.stringify({
      version: 1,
      window: {
        sync_mode: "incremental",
        phase: "bounded",
        base_anchor: JSON.parse(anchor(1)),
        has_reached_candidate: false,
        has_seen_base: false,
      },
      source: {
        kind: "crossref",
        issn: "1234-5679",
        cursor: "stale",
        page_index: 9,
        cursor_refreshed_at_epoch_seconds: 1,
      },
    }),
  },
);
const directory = await fs.mkdtemp(
  path.resolve("output/migration/execution/index-flows-"),
);
const result = spawnSync(
  "output/migration/execution/sources-index-oracle.exe",
  [],
  {
    input:
      cases
        .map((value, index) =>
          JSON.stringify({
            ...value,
            root: path.join(directory, String(index)),
          }),
        )
        .join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => {
    const value = JSON.parse(line);
    delete value.root;
    return value;
  });
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/index-flow-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-index-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-index-flows.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust complete index workflows`,
);
