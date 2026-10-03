/** Freeze original Crossref metadata and consumed-payload identity without touching database files. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const cases = [];
/** Register raw JSON without routing its integer values through JavaScript decoding. */
function add(kind, input) {
  cases.push({
    id: `${kind}-${cases.length}`,
    kind,
    input: typeof input === "string" ? input : JSON.stringify(input),
  });
}
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
const collect = {
  ...initial,
  created_from: 0,
  phase: {
    kind: "collect",
    partition: 1,
    from: 0,
    until: 1800000000,
    cursor: null,
    received: 0,
    expected: null,
    retry: 0,
  },
};
const ready = { ...collect, root_total: 0, phase: { kind: "ready" } };
const emit = {
  ...ready,
  phase: { kind: "emit", upper: "2026-01-01", lower: null, after: null },
};
const anchors = [
  {
    version: 1,
    issue: {
      kind: "volume_issue",
      publication_year: 2026,
      volume: "12",
      issue: "3",
    },
    from_sync_date: "2026-01-01",
  },
  { version: 1, issue: { kind: "date", date: "2026-02" } },
  {
    version: 1,
    issue: { kind: "title", publication_year: null, title: "special issue" },
  },
];
for (const state of [initial, collect, ready, emit]) {
  add("checkpoint", state);
  add("checkpoint", Object.values(state));
  for (const name of Object.keys(state)) {
    const missing = { ...state };
    delete missing[name];
    add("checkpoint", missing);
    add("checkpoint", { ...state, [name]: null });
    add(
      "checkpoint",
      JSON.stringify(state).replace(
        /}$/,
        `,"${name}":${JSON.stringify(state[name])}}`,
      ),
    );
  }
  add("checkpoint", { ...state, extra: 1 });
  add("checkpoint", { ...state, phase: { ...state.phase, extra: 1 } });
  add("checkpoint", { ...state, phase: Object.values(state.phase) });
  add("checkpoint", { ...state, phase: [...Object.values(state.phase), 0] });
  add(
    "checkpoint",
    JSON.stringify(state).replace(
      '"kind":',
      `"kind":"${state.phase.kind}","kind":`,
    ),
  );
  for (const token of [
    "",
    "A".repeat(32),
    "f".repeat(31),
    "f".repeat(33),
    "f".repeat(32),
  ])
    add("checkpoint", { ...state, token });
  for (const counter of [
    "-1",
    "-0",
    "1.0",
    "1e0",
    "2",
    "255",
    "256",
    "18446744073709551615",
    "18446744073709551616",
  ]) {
    for (const name of ["generation", "sequence"]) {
      add(
        "checkpoint",
        JSON.stringify(state).replace(`"${name}":0`, `"${name}":${counter}`),
      );
    }
  }
  for (const frozen of [
    "-8334601228801",
    "-8334601228800",
    "8210266876799",
    "8210266876800",
  ]) {
    add(
      "checkpoint",
      JSON.stringify(state).replace(
        '"frozen_at":1800000000',
        `"frozen_at":${frozen}`,
      ),
    );
  }
  for (const anchor of anchors)
    add("checkpoint", { ...state, candidate: anchor });
}
for (const phase of [
  {
    kind: "collect",
    partition: 0,
    from: 0,
    until: 0,
    cursor: null,
    received: 0,
    expected: null,
    retry: 0,
  },
  {
    kind: "collect",
    partition: 2,
    from: 0,
    until: 0,
    cursor: null,
    received: 0,
    expected: null,
    retry: 0,
  },
  ...[225, 226, 300].flatMap((expected) =>
    ["", "*", null].map((cursor) => ({
      kind: "collect",
      partition: 1,
      from: 0,
      until: 0,
      cursor,
      received: expected,
      expected,
      retry: 1,
    })),
  ),
  { kind: "emit", upper: "2026-01-01", lower: "2027-01-01", after: null },
  {
    kind: "emit",
    upper: "2026-01-01",
    lower: null,
    after: { rank: 1, date: "", key: "doi:x" },
  },
  {
    kind: "emit",
    upper: "2026-01-01",
    lower: null,
    after: { rank: 0, date: "2026-01-01", key: "doi:x" },
  },
  { kind: "other" },
  ["discover"],
])
  add("checkpoint", { ...collect, phase });
for (const anchor of anchors) {
  add("anchor", anchor);
  add("anchor", { ...anchor, issue: Object.values(anchor.issue) });
  add("anchor", { ...anchor, extra: 1 });
  add("anchor", { ...anchor, version: 2 });
  for (const from_sync_date of [null, "2026", "2026-02-29", "0000-01-01"]) {
    add("anchor", { ...anchor, from_sync_date });
  }
}
for (const issue of [
  { kind: "volume_issue", publication_year: 0, volume: "1" },
  { kind: "volume_issue", publication_year: 2026, volume: "01" },
  { kind: "volume_issue", publication_year: 2026, volume: null, issue: null },
  { kind: "volume_issue", publication_year: 2026, volume: "" },
  { kind: "date", date: "0000" },
  { kind: "date", date: "2026-02-29" },
  { kind: "title", title: "Not Normalized" },
  { kind: "title", title: "" },
  { kind: "title", publication_year: 0, title: "valid title" },
])
  add("anchor", { version: 1, issue });
const work = {
  DOI: " HTTPS://DOI.ORG/10.1234/AbC ",
  title: ["Article"],
  volume: "0012",
  issue: "03",
  created: { timestamp: 1800000000123, other: 1 },
  "published-online": { "date-parts": [[2026, 2, 3]], other: 1 },
  author: [{ given: "A", family: "B", ORCID: "discard" }, null],
  "updated-by": [{ DOI: "10.1/X", type: "correction", other: 1 }],
  ignored: "discard",
};
for (const value of [
  null,
  [],
  {},
  work,
  { ...work, DOI: null },
  { ...work, DOI: "invalid" },
  { ...work, DOI: 3.5 },
])
  add("work", value);
for (const timestamp of [
  "-9223372036854775808",
  "9223372036854775807",
  "-1001",
  "-1000",
  "-999",
  "-1",
  "0",
  "1",
  "1.0",
  "1e3",
  "null",
  '"123"',
])
  add(
    "work",
    JSON.stringify(work).replace(
      '"timestamp":1800000000123',
      `"timestamp":${timestamp}`,
    ),
  );
for (const parts of [
  [0],
  [1],
  [9999],
  [10000],
  [2026, 2, 29],
  [2024, 2, 29],
  [2026, null, 3],
  [2026, 0],
  [2026, 12, 31],
  [],
  ["2026"],
])
  add("work", {
    ...work,
    "published-online": { "date-parts": [parts] },
    "published-print": { "date-parts": [[2025, 1, 1]] },
  });
for (const volume of [
  "",
  "000",
  "+1",
  "1.0",
  "18446744073709551615",
  "18446744073709551616",
  " 12 ",
  "Part A",
  null,
  12,
])
  add("work", { ...work, volume });
for (const number of ["1e20", "1e-6", "-0", "18446744073709551615"]) {
  add(
    "work",
    `{"title":["numeric"],"volume":${number},"created":{"timestamp":0},"author":[{"given":${number}}]}`,
  );
}
const result = spawnSync(
  "output/migration/execution/sources-workset-oracle.exe",
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
  "tests/migration/sources/workset-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-workset-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-workset.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust workset observations`);
