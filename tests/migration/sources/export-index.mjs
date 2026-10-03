/** Freeze strict index checkpoints, frozen-context validation and issue-window planning. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const cases = [];
/** Retain exact JSON source text where integer or duplicate-field representation matters. */
function add(kind, input) {
  cases.push({
    id: `${kind}-${cases.length}`,
    kind,
    input: typeof input === "string" ? input : JSON.stringify(input),
  });
}
/** Produce a normalized issue boundary with independently serialized sync metadata. */
function anchor(issue, year = 2026) {
  return {
    version: 1,
    issue: {
      kind: "volume_issue",
      publication_year: year,
      volume: "1",
      issue: String(issue),
    },
    from_sync_date: `${year}-01-01`,
  };
}
const base = anchor(2),
  candidate = anchor(3);
const window = {
  sync_mode: "incremental",
  phase: "bounded",
  base_anchor: base,
  candidate_anchor: null,
  has_reached_candidate: false,
  has_seen_base: false,
};
const initial = {
  token: "0123456789abcdef0123456789abcdef",
  issn: "1234-5679",
  frozen_at: 1800000000,
  created_from: null,
  updated_from: null,
  root_total: null,
  candidate: null,
  generation: 0,
  sequence: 0,
  phase: { kind: "discover" },
};
const sources = [
  { kind: "crossref_workset", state: initial },
  {
    kind: "crossref",
    issn: "1234-5679",
    cursor: null,
    page_index: 0,
    cursor_refreshed_at_epoch_seconds: null,
  },
  { kind: "open_alex", source_id: "S1", cursor: null },
];
for (const [field, variant] of [
  ["sync_mode", "incremental"],
  ["phase", "bounded"],
]) {
  for (const representation of [
    `{"${variant}":null}`,
    `{"${variant}":false}`,
    `{"${variant}":[]}`,
    `{"${variant}":null,"${variant}":null}`,
    `{"${variant}":null,"other":null}`,
    `{}`,
  ]) {
    const raw = JSON.stringify({ version: 2, window, source: sources[2] });
    add(
      "checkpoint",
      raw.replace(`"${field}":"${variant}"`, `"${field}":${representation}`),
    );
  }
}
add("checkpoint", {
  version: 2,
  window: {
    ...window,
    sync_mode: { incremental: null },
    phase: { bounded: null },
  },
  source: sources[2],
});
for (const source of sources)
  for (const version of [0, 1, 2, 3]) {
    const state = { version, window, source };
    add("checkpoint", state);
    add("checkpoint", Object.values(state));
    for (const layer of [null, "window", "source"]) {
      const object = layer ? state[layer] : state;
      for (const name of Object.keys(object))
        for (const replacement of [undefined, null]) {
          const changed = structuredClone(state),
            target = layer ? changed[layer] : changed;
          if (replacement === undefined) delete target[name];
          else target[name] = replacement;
          add("checkpoint", changed);
        }
      const changed = structuredClone(state);
      (layer ? changed[layer] : changed).unknown = true;
      add("checkpoint", changed);
    }
    add("checkpoint", { ...state, source: Object.values(source) });
  }
for (const mode of ["bootstrap", "incremental", "full_rescan", "other"])
  for (const phase of ["bounded", "unbounded", "other"])
    add("checkpoint", {
      version: 2,
      window: { ...window, sync_mode: mode, phase },
      source: sources[2],
    });
for (const selectedBase of [
  null,
  base,
  anchor(2, 2099),
  { ...base, from_sync_date: null },
  { ...base, version: 2 },
])
  for (const selectedCandidate of [null, candidate])
    for (const reached of [false, true])
      for (const seen of [false, true])
        for (const phase of ["bounded", "unbounded"]) {
          add("checkpoint", {
            version: 2,
            window: {
              ...window,
              phase,
              base_anchor: selectedBase,
              candidate_anchor: selectedCandidate,
              has_reached_candidate: reached,
              has_seen_base: seen,
            },
            source: sources[2],
          });
        }
for (const cursor of [null, "", "opaque"])
  for (const page_index of [0, 1])
    for (const refreshed of [null, 0, 1])
      add("checkpoint", {
        version: 1,
        window,
        source: {
          ...sources[1],
          cursor,
          page_index,
          cursor_refreshed_at_epoch_seconds: refreshed,
        },
      });
for (const key of ["page_index", "cursor_refreshed_at_epoch_seconds"])
  for (const value of [
    "-1",
    "-0",
    "1.0",
    "18446744073709551615",
    "18446744073709551616",
  ])
    add(
      "checkpoint",
      JSON.stringify({
        version: 1,
        window,
        source: {
          ...sources[1],
          cursor: "opaque",
          cursor_refreshed_at_epoch_seconds: 0,
        },
      }).replace(`"${key}":0`, `"${key}":${value}`),
    );
add("checkpoint", " ".repeat(65537));
const patterns = [
  [],
  [null],
  [anchor(3)],
  [anchor(2)],
  [anchor(1)],
  [anchor(3), anchor(3), anchor(2), anchor(2), anchor(1)],
  [anchor(4), anchor(3), anchor(2)],
  [anchor(3), null, anchor(2)],
  [anchor(3), anchor(4), anchor(2)],
  [anchor(2), anchor(3)],
  [anchor(3, 2025)],
  [{ version: 1, issue: { kind: "date", date: "2026-02" } }],
];
for (const phase of ["bounded", "unbounded"])
  for (const fixedCandidate of [null, candidate])
    for (const reached of [false, true])
      for (const seen of [false, true]) {
        if (
          (!fixedCandidate && (reached || seen)) ||
          (seen && (!reached || phase !== "bounded"))
        )
          continue;
        const frozen = {
          ...window,
          phase,
          candidate_anchor: fixedCandidate,
          has_reached_candidate: reached,
          has_seen_base: seen,
        };
        for (const anchors of patterns)
          for (const unknown of [false, true])
            for (const next of [false, true])
              add("window", { window: frozen, anchors, unknown, next });
      }
for (const mode of ["bootstrap", "incremental", "full_rescan"])
  for (const committed of [
    null,
    "bad",
    JSON.stringify(base),
    JSON.stringify(anchor(2, 2099)),
  ])
    for (const checkpoint of [
      null,
      JSON.stringify({ version: 2, window, source: sources[2] }),
    ])
      add("context", { mode, anchor: committed, checkpoint });
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
for (const title of [
  "Journal",
  "A < B & C > D",
  "literal \\u003c",
  "中文\u2028title",
])
  add("scope", {
    catalog: {
      ...catalog,
      title,
      title_aliases: [title],
      rankings: { ...catalog.rankings, utd_rank: title },
    },
    window,
  });
const worksetCorpus = JSON.parse(
  await fs.readFile("tests/migration/sources/workset-vectors.json", "utf8"),
);
for (const value of worksetCorpus.observations.filter(
  (value) => value.kind === "work",
))
  add(
    "article",
    `{"provider":"crossref","catalog":${JSON.stringify(catalog)},"work":${value.input}}`,
  );
const crossref = {
  DOI: "10.1234/test",
  title: ["Title"],
  published: { "date-parts": [[2026, 1, 1]] },
  volume: "1",
  issue: "2",
  page: "1-9",
  author: [{ given: "A", family: "B" }],
  abstract: "<p>text</p>",
};
for (const title of [undefined, null, [], [null, ""], [], [""], "", true, 123])
  for (const doi of [null, "invalid", "10.1234/test"]) {
    const work = { ...crossref, title, DOI: doi };
    add("article", { provider: "crossref", catalog, work });
    for (const matches of [false, true])
      add("article", {
        provider: "crossref",
        catalog,
        work,
        openalex: {
          doi: matches ? doi : "10.1234/other",
          display_name: "OA title",
          best_oa_location: null,
          abstract_inverted_index: { word: [0] },
        },
        semantic_scholar: {
          externalIds: { DOI: matches ? doi : "10.1234/other" },
          title: "S2 title",
          abstract: "S2 abstract",
          isOpenAccess: false,
        },
      });
  }
for (const index of [
  { z: [0], a: [0], b: [1] },
  { word: [-1, 1] },
  { word: [1.5] },
  { word: null },
  { word: ["1"] },
  {},
])
  add("article", {
    provider: "crossref",
    catalog,
    work: { DOI: "10.1234/test" },
    openalex: { abstract_inverted_index: index },
  });
const openalex = {
  display_name: "Title",
  publication_date: "2026-01-01",
  publication_year: 2026,
  doi: "https://doi.org/10.1234/test",
  biblio: { volume: "1", issue: "2", first_page: "1", last_page: "9" },
  authorships: [{ author: { display_name: "A" } }],
  open_access: { is_oa: true },
  abstract_inverted_index: { hello: [0], world: [1] },
};
for (const key of Object.keys(openalex))
  for (const value of [undefined, null, "", 0, false, {}, []])
    add("article", {
      provider: "openalex",
      catalog,
      work: { ...openalex, [key]: value },
    });
for (const work of [
  { title: "Title" },
  { title: "Title", publication_year: 2026, biblio: { volume: "1" } },
  {
    title: "Title",
    publication_date: "invalid",
    biblio: { volume: "1" },
    publication_year: -1,
  },
  { title: "Title", doi: "10.1234/test" },
])
  add("article", { provider: "openalex", catalog, work });
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
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/index-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-index-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-index.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust index observations`);
