/** Freeze original CFP storage transitions and domain decisions from the locked Rust observer. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const source = {
  catalogIds: ["a", "alias-a"],
  journalTitle: "Original Journal",
  title: "Original special issue",
  typeText: "Special Issue",
  dateText: "Submission deadline: 31 December 2026",
  sourceUrl: "https://example.org/cfp",
  checkedOn: "2026-09-15",
};
/** Encode exact imported bytes so whitespace remains part of the import identity. */
function seed(sources = [source], emptyJournals = [], extra = {}) {
  return JSON.stringify({ formatVersion: 1, sources, emptyJournals, ...extra });
}
const initial = { op: "import", payload: seed() },
  begin = { op: "begin" },
  publish = { op: "publish", sources: [source] };
const cases = [];
/** Add one independent durable transition history. */
function add(name, steps) {
  cases.push({ name, steps });
}
add("immutable-seed", [
  initial,
  initial,
  { ...initial, payload: seed() + " " },
  { op: "load" },
  { op: "originals" },
]);
add("refresh-and-reimport", [
  initial,
  begin,
  {
    ...publish,
    sources: [
      { ...source, scope: "new scope", journalTitle: "Ignored renamed title" },
    ],
  },
  initial,
  { op: "load" },
]);
add("active-generation-replaced", [
  initial,
  begin,
  { op: "begin", now: 101 },
  publish,
  { ...publish, generation: 2, expires: 111 },
  { op: "load" },
]);
for (const now of [109, 110, 111])
  add(`expiry-${now}`, [initial, begin, { ...publish, now }, { op: "load" }]);
for (const expires of [100, 105, 106, 200])
  add(`token-expiry-${expires}`, [
    initial,
    begin,
    { ...publish, expires },
    { op: "load" },
  ]);
for (const seconds of [0, -1, 1, 600, 601])
  add(`lease-seconds-${seconds}`, [
    initial,
    { ...begin, seconds },
    { op: "load" },
  ]);
add("overflow-lease", [
  initial,
  { op: "begin", now: JSON.rawJSON("9223372036854775807"), seconds: 1 },
]);
add("missing-source", [begin, { op: "fail" }, publish, { op: "load" }]);
add("expired-fail-retains-data", [
  initial,
  begin,
  publish,
  { op: "begin", now: 200 },
  { op: "fail", generation: 2, expires: 210, reason: "界😀".repeat(700) },
  { op: "fail", generation: 2 },
  { op: "load" },
]);
add("unsupported-stale-fail", [
  initial,
  begin,
  { op: "begin" },
  { op: "fail", unsupported: true },
  { op: "fail", generation: 2, unsupported: true },
  { op: "load" },
]);
for (const aliases of [["a"], ["a", "new-alias"], ["alias-a", "a"]])
  add(`aliases-${aliases.join("-")}`, [
    initial,
    begin,
    { ...publish, sources: [{ ...source, catalogIds: aliases }] },
    { op: "load" },
  ]);
for (const unresolved of [0, 1, 2])
  add(`full-partial-${unresolved}`, [
    initial,
    begin,
    publish,
    { op: "begin", now: 200 },
    {
      op: "full",
      generation: 2,
      expires: 210,
      now: 205,
      unresolved,
      sources: [
        {
          ...source,
          scope: "full original ".repeat(200),
          sourceUrl: "https://example.org/full",
        },
      ],
    },
    { op: "load" },
  ]);
for (const change of [
  { title: "Different" },
  { catalogIds: ["alias-a", "a"] },
  { catalogIds: ["a"] },
])
  add(`full-identity-${JSON.stringify(change)}`, [
    initial,
    begin,
    { op: "full", sources: [{ ...source, ...change }], capture: "" },
    { op: "load" },
  ]);
add("full-drop-rejected", [
  initial,
  begin,
  { op: "full", sources: [] },
  { op: "load" },
]);
const empty = {
  catalogIds: ["a", "alias-a"],
  journalTitle: "Original Journal",
  checkedOn: "2026-10-01",
  sourceUrl: "https://example.org/none",
  sourceStatement: "No calls listed",
};
add("verified-empty", [
  initial,
  begin,
  { op: "publish", empty: [empty] },
  { op: "load" },
  { op: "originals" },
]);
add("empty-seed", [
  { op: "import", payload: seed([], [empty]) },
  { op: "load" },
]);
add("no-implicit-clear", [initial, begin, { op: "publish" }, { op: "load" }]);
for (const payload of [
  "{invalid}",
  seed([], [], { formatVersion: 2 }),
  seed([source, source]),
  seed([source, { ...source, catalogIds: ["b", "alias-a"] }]),
  seed([source], [empty]),
  seed([source], [], { expectedJournals: 2 }),
  seed([source], [], { expectedNotices: 0 }),
  seed([{ ...source, journalTitle: "" }]),
  seed([], [{ ...empty, checkedOn: "2026-1-2" }]),
  seed([], [{ ...empty, checkedOn: "2026-02-30" }]),
  seed([], [{ ...empty, notices: [{}] }]),
])
  add(`validation-${cases.length}`, [
    { op: "import", payload },
    { op: "load" },
  ]);
add("whole-import-rollback", [
  initial,
  {
    op: "import",
    id: "collision",
    payload: seed([{ ...source, catalogIds: ["0-first"] }, source]),
  },
  { op: "load" },
]);
add("publish-rollback-trigger", [
  initial,
  begin,
  {
    ...publish,
    sql: "CREATE TRIGGER reject_notice BEFORE INSERT ON cfp_notices BEGIN SELECT RAISE(ABORT,'fixture'); END;",
  },
  { op: "load" },
]);
add("expiry-after-update-rollback", [
  initial,
  begin,
  { ...publish, expires: 104 },
  { op: "load" },
]);
add("strict-state-integer", [
  initial,
  { op: "load", sql: "UPDATE cfp_sources SET revision='corrupt'" },
]);
add("strict-notice-json", [
  initial,
  { op: "load", sql: "UPDATE cfp_notices SET normalized_json='{}'" },
]);
add("strict-original-json", [
  initial,
  { op: "originals", sql: "UPDATE cfp_notices SET source_json='{}'" },
]);
add("strict-import-hash", [
  initial,
  { ...initial, sql: "UPDATE cfp_seed_imports SET content_hash=x'ff'" },
]);
for (const mutation of [
  { capture: "" },
  { url: "file:///private" },
  { sources: [] },
  { sources: [{ ...source, dateText: "Deadline: 31 October" }] },
])
  add(`validation-before-stale-${cases.length}`, [
    initial,
    begin,
    { ...publish, ...mutation, generation: 99 },
    { op: "load" },
  ]);
const domain = [];
/** Add an independent pure domain observation. */
function observe(name, input) {
  domain.push({ name, input: { mode: "domain", ...input } });
}
const times = [
  "2026-09-19T15:59:59Z",
  "2026-09-19T16:00:00Z",
  "2026-09-20T10:00:00Z",
  "2026-09-21T12:00:00Z",
  "2027-01-01T12:00:00Z",
];
const texts = [
  "",
  "Submission deadline: 31 December 2026",
  "论文投稿截止日期 2026 年 12 月 31 日",
  "Submission deadline\n31 December 2026",
  "December 31, 2026: Manuscript submission deadline",
  "Submission deadline: Sept. 15, 2026",
  "31, March, 2023: Paper submission deadline",
  "Final submission deadline: 31^{st} March 2027.",
  "Deadline for Submissions: 31/01/2027",
  "Submission Deadline: October 30, 2026Contribute to this Special Topic",
  "Submission deadline: February 29, 2028",
  "Submissions Sept 1 - 30, 2026",
  "Submission period: April 30, 2026 - October 31, 2026",
  "Submission window: 15 January 2027–12 February 2027",
  "Submit proposals between January 1 and January 15, 2027",
  "Submission window: 1–31 December 2026",
  "Submission window: 15 January – 15 February 2027",
  "Submission window: December 15–January 15, 2027",
  "Submission deadline: 31 October",
  "Submission deadline: 31 June 2027",
  "Submission deadline: February 29, 2027",
  "Submission deadline: autumn 2027",
  "Submission deadline: 2026-12-310",
  "Submission deadline: 12026-12-31",
  "Submission deadline: 1 May 2027\nSubmission deadline: 1 June 2027",
  "Submission window: 1–31 December 20260",
  "Submission deadlines: January 1 and January 15, 2027",
  "Submission deadline: 01/02/2027",
  "Abstract deadline: TBD\nFull manuscript submission deadline: 1 December 2026",
  "Submission deadline: 15 September 2026\nSubmission deadline (now extended!): October 1, 2026",
  "Optional proposal deadline: 1 May 2026\nFull manuscript submission deadline: 1 December 2026",
  "应征作者务请于2026年9月20日前将论文发送至邮箱",
  "Deadline before Friday, September 20, 2026",
  "deadline界by界 20 September 2026",
  "Deadline:界September 20, 2026",
  "Submission deadline:\u200331\u00a0December\u00852026",
  "Submission deadline: ２０２６年１２月３１日",
  "Registration: 2026-02-30\nFull paper due: 2026-12-31",
  "Opens: TBD\nSubmission deadline: 2026-12-31",
  "Abstract due: 2026-01-01\nPaper due: 2026-12-31",
  "Publication: 2028-01-01. Submission deadline: 2026-12-31",
];
for (const stage of ["paper", "proposal", "abstract"])
  for (const text of texts)
    observe(`dates-${stage}-${domain.length}`, { op: "dates", stage, text });
for (const change of [
  {},
  { timeZone: "UTC" },
  { timeZone: "Asia/Shanghai" },
  { timeZone: "Factory" },
  { timeZone: "Local" },
  { timeZone: "invalid" },
  { isHistorical: true },
  { statusText: "Closed" },
  { statusText: "界closed界" },
  { statusText: "Invitation only" },
  { rawDateText: "December 2030" },
  { entryStage: "opens" },
  { checkedOn: "2026-9-15" },
  { checkedOn: "2026-02-30" },
  { checkedOn: "0000-01-01" },
  { title: "  Just a moment  " },
  { title: "年度重点选题2027", typeText: "General" },
  { sourceUrl: "https://user:pass@example.org/x" },
  { sourceUrl: "https:example.org/x" },
  { sourceUrl: "https://example.org/" },
  { catalogIds: [] },
  { journalTitle: "" },
  { dateText: "Deadline: 31 October", isHistorical: true },
])
  observe(`source-${domain.length}`, {
    op: "source",
    raw: JSON.stringify({
      ...source,
      dateText: "Submission deadline: 20 September 2026",
      ...change,
    }),
    times,
  });
for (const field of Object.keys(source)) {
  const copy = { ...source };
  delete copy[field];
  observe(`missing-${field}`, {
    op: "source",
    raw: JSON.stringify(copy),
    times,
  });
  observe(`null-${field}`, {
    op: "source",
    raw: JSON.stringify({ ...source, [field]: null }),
    times,
  });
}
for (const raw of [
  JSON.stringify({ ...source, extra: 1 }),
  JSON.stringify(source).replace('"title":', '"title":"duplicate","title":'),
  JSON.stringify({ ...source, scope: null }),
  JSON.stringify({ ...source, isHistorical: null }),
  JSON.stringify({ ...source, entryStage: { paper: null } }),
  JSON.stringify({ ...source, entryStage: { paper: null, proposal: null } }),
  JSON.stringify({ ...source, entryStage: "invalid" }),
  JSON.stringify({ ...source, scope: "\ud800" }),
])
  observe(`wire-${domain.length}`, { op: "source", raw, times });

for (const checkedOn of [
  "1-1-1",
  "+2026-1-2",
  "-0001-1-2",
  " 2026- 1- 2",
  "2026-01-02 ",
  "262143-01-01",
  "+262143-01-01",
])
  add(`empty-date-${checkedOn}`, [
    { op: "import", payload: seed([], [{ ...empty, checkedOn }]) },
    { op: "load" },
  ]);
observe("unicode-window-year", {
  op: "dates",
  text: "Submission window May 1–2 ٢٠٢٦",
  stage: "paper",
});
observe("unicode-topic-year", {
  op: "source",
  raw: JSON.stringify({
    ...source,
    title: "年度20٢٦选题",
    typeText: "general",
    dateText: "",
  }),
  times,
});
const positional = [
  source.catalogIds,
  source.journalTitle,
  source.title,
  "",
  "",
  source.typeText,
  source.dateText,
  source.sourceUrl,
  source.checkedOn,
  null,
  null,
  null,
  false,
  "",
];
for (const length of [11, 12, 13, 14])
  observe(`source-array-${length}`, {
    op: "source",
    raw: JSON.stringify(positional.slice(0, length)),
    times,
  });
observe("empty-default-array", {
  op: "seed",
  raw: JSON.stringify({
    formatVersion: 1,
    sources: [],
    emptyJournals: [
      [
        empty.catalogIds,
        empty.journalTitle,
        empty.checkedOn,
        empty.sourceUrl,
        empty.sourceStatement,
      ],
    ],
  }),
});
const notice = {
  id: "id",
  title: "title",
  scope: "",
  requirements: "",
  kind: "special_issue",
  sourceUrl: source.sourceUrl,
  checkedOn: source.checkedOn,
  dates: [],
  entryStage: "paper",
  topicYear: null,
  timeZone: null,
  sourceStatus: null,
  isHistorical: false,
  rawDateText: "",
};
for (const extra of ["1e400", '"\\ud800"', '{"nested":1e400}'])
  observe(`notice-ignored-${extra}`, {
    op: "notice",
    raw: JSON.stringify(notice).slice(0, -1) + ',"extra":' + extra + "}",
  });

const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/cfp-oracle/storage-build.json",
    "utf8",
  ),
);
const directory = await fs.mkdtemp(
  path.resolve("output/migration/execution/cfp-oracle/histories-"),
);
const requests = [
  ...cases.map((entry, index) => ({
    path: path.join(directory, `${index}.sqlite`),
    steps: entry.steps,
  })),
  ...domain.map((entry) => entry.input),
];
const result = spawnSync(build.binary, [], {
  input:
    requests
      .map((entry) =>
        JSON.stringify(entry).replace(
          '"now":"9223372036854775807"',
          '"now":9223372036854775807',
        ),
      )
      .join("\n") + "\n",
  encoding: "utf8",
  timeout: 120000,
  maxBuffer: 96 * 1024 * 1024,
  windowsHide: true,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const lines = result.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, requests.length);
for (let index = 0; index < cases.length; index++)
  cases[index].expected = JSON.parse(lines[index]);
for (let index = 0; index < domain.length; index++)
  domain[index].expected = JSON.parse(lines[cases.length + index]);
assert.equal(cases[0].expected[0].outcome.didImport, true);
assert.equal(cases[0].expected[1].outcome.didImport, false);
await fs.writeFile(
  "tests/migration/cfp/storage-vectors.json",
  JSON.stringify(
    {
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/cfp/export-storage.mjs"),
      ),
      cases,
      domain,
    },
    null,
    2,
  ) + "\n",
);
console.log({
  histories: cases.length,
  transitions: cases.reduce((sum, entry) => sum + entry.steps.length, 0),
  domain: domain.length,
});
