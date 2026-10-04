/** Freeze independently computed recommendation observations without rounding large JSON integers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
/** Register one JSON input with an explicit stable case name. */
function add(name, input) {
  cases.push({ name, input: JSON.stringify(input) });
}
/** Construct one complete candidate independent of implementation defaults. */
function candidate(id, overrides = {}) {
  return {
    article_id: id,
    journal_id: 1,
    issue_id: 2,
    title: `Rust ${id}`,
    abstract_text: "systems",
    date: "2026-10-04",
    journal_title: "Journal",
    doi: null,
    open_access: false,
    in_press: false,
    ...overrides,
  };
}
/** Wrap an arbitrary model message in the original first-choice envelope. */
function response(message) {
  return { choices: [{ message }] };
}
const payloads = [
  null,
  true,
  1,
  "",
  "plain text",
  [],
  [" first ", null, "second"],
  {},
  { summary: " summary " },
  { summary: "", message: " alternative " },
  { only: " unique " },
  { a: "x", b: "y" },
  { selected: [] },
  {
    selected: [
      1,
      "+2",
      "03",
      1.0,
      [4, "2.5"],
      { article_id: 5, extra: 1 },
      null,
      true,
    ],
  },
  { selected: { 2: "3.5", 10: 4, bad: 3, "+1": "NaN", "03": "1e999" } },
  { items: [6] },
  { results: [7] },
  { recommendations: [8] },
  { articles: [9] },
  { article_id: "+10", score: "inf" },
  { article_id: 11.5 },
  { selected: null, items: [12] },
  { summary: "top", reason: "bottom", selected: [1] },
  { summary: 22, text: "text", selected: [1] },
];
for (const kind of ["selection", "summary"]) {
  for (const [index, payload] of payloads.entries()) {
    add(`${kind}-parsed-${index}`, {
      op: "payload",
      kind,
      response: response({ parsed: payload }),
    });
    add(`${kind}-content-${index}`, {
      op: "payload",
      kind,
      response: response({ content: JSON.stringify(payload) }),
    });
  }
  for (const [index, message] of [
    { parsed: null, content: { selected: [1] } },
    { refusal: " no ", parsed: { selected: [1] } },
    { refusal: 1, content: { selected: [1] } },
    {
      content: [{ text: '{"summary":' }, { text: '"joined","selected":[1]}' }],
    },
    { content: '```json\r\n{"summary":"fenced","selected":[1]}\r\n```' },
    { content: "```" },
    { content: "```x\nplain\n```tail" },
    { content: 42 },
    { content: '"\\ud800"' },
  ].entries())
    add(`${kind}-shape-${index}`, {
      op: "payload",
      kind,
      response: response(message),
    });
}
for (const [index, value] of [
  null,
  {},
  { choices: [] },
  { choices: [null] },
  { choices: [{}] },
  { choices: [{ message: null }] },
].entries())
  add(`invalid-envelope-${index}`, {
    op: "payload",
    kind: "selection",
    response: value,
  });
for (const raw of [
  "9223372036854775807",
  "9223372036854775808",
  "-9223372036854775808",
  "-0",
  "1.0",
  "1e0",
])
  cases.push({
    name: `number-${raw}`,
    input: `{"op":"payload","kind":"selection","response":{"choices":[{"message":{"parsed":{"selected":[${raw},[${raw},3]]}}}]}}`,
  });
const candidates = Array.from({ length: 25 }, (_, index) =>
  candidate(index + 1, {
    title: index % 2 ? "Rust" : "unrelated",
    abstract_text: index % 3 ? "systems" : "other",
  }),
);
const subscriber = {
  subscriber_id: "sub",
  name: "Reader",
  keywords: ["rust", " rust ", ""],
  directions: ["systems"],
};
for (let count = 0; count <= 25; count++)
  add(`selection-count-${count}`, {
    op: "selection",
    subscriber,
    candidates,
    selection: {
      summary: "",
      selections: Array.from({ length: count }, (_, index) => ({
        article_id: index + 1,
        score: 100 - index,
      })),
    },
    dedupe: { "sub:3": "", "sub:8": "sent" },
  });
for (const [name, settings] of Object.entries({
  duplicates: [
    { article_id: 2, score: 1 },
    { article_id: 2, score: 2 },
    { article_id: 999, score: 99 },
  ],
  stable: [
    { article_id: 4, score: 1.0000001 },
    { article_id: 2, score: 1.0000002 },
  ],
  saturation: [
    { article_id: 4, score: 1e30 },
    { article_id: 2, score: 1e25 },
    { article_id: 6, score: -1e30 },
  ],
}))
  add(name, {
    op: "selection",
    subscriber,
    candidates,
    selection: { summary: "", selections: settings },
    dedupe: {},
  });
add("unicode-match", {
  op: "selection",
  subscriber: {
    subscriber_id: "s",
    keywords: ["ΟΣ", "i̇", "ẞ"],
    directions: ["σ"],
  },
  candidates: [
    candidate(1, { title: "ΟΣ İ ß", abstract_text: "" }),
    candidate(1, { title: "duplicate", abstract_text: "σ" }),
  ],
  selection: { summary: "", selections: [{ article_id: 1, score: 1 }] },
  dedupe: {},
});
for (const [index, overrides] of [
  {},
  { ai_api_key: "" },
  { ai_model: " " },
  { ai_base_url: "https://a.test/v1/" },
  { ai_base_url: " https://a.test/v1 " },
  { ai_backup_api_key: "key" },
  { ai_backup_base_url: "https://b.test/v1" },
  { ai_backup_model: "different" },
  { ai_backup_api_key: " " },
  { ai_system_prompt: " ", ai_backup_system_prompt: "backup" },
].entries())
  for (const override of [null, "", "override"])
    add(`config-${index}-${override}`, {
      op: "config",
      subscriber: overrides,
      global: {
        ai_base_url: "https://a.test/v1",
        ai_allowed_base_urls: ["https://a.test/v1", "https://b.test/v1"],
        ai_api_key: "key",
        ai_system_prompt: "global",
      },
      defaults: { ai_model: "default" },
      override,
    });
for (const [index, path] of [
  "db",
  "db.sqlite",
  "db.SQLITE",
  "",
  " ",
  ".",
  "..",
  "/",
  "/tmp/db/.",
  "folder/db/",
  "C:\\data\\db.sqlite",
  "C:",
  "a/../db",
].entries())
  add(`database-${index}`, {
    op: "database",
    database: path,
    selected: ["db"],
  });
add("snapshot", {
  op: "snapshot",
  previous: { "1:2": 1, deleted: 3 },
  current: {
    "1:2": 1,
    "01:2": 3,
    "+1:2": 4,
    "-1:2": 1,
    "2:1": 0,
    bad: 2,
    4: 1,
    "+4": 2,
    "04": 3,
  },
});
for (const [name, rows, summary] of [
  ["normal", candidates, " overview "],
  [
    "missing-fields",
    [candidate(1, { title: " ", abstract_text: " ", doi: "", date: "" })],
    "",
  ],
  [
    "skip-large-section",
    [candidate(1, { abstract_text: "中".repeat(7000) }), candidate(2)],
    "",
  ],
  ["oversized-header", [], "中".repeat(19000)],
])
  add(`message-${name}`, {
    op: "message",
    subscriber,
    candidates: rows,
    selection: {
      summary,
      selections: rows.map((row) => ({ article_id: row.article_id, score: 1 })),
    },
    database: "db.sqlite",
    run: "中文😀123456789long",
  });
for (const score of [
  "+NaN",
  "-NaN",
  "+nan",
  "-nAn",
  "--NaN",
  "+-NaN",
  " NaN",
  "+Infinity",
  "1_0",
  "0x1p2",
])
  add(`score-string-${score}`, {
    op: "payload",
    kind: "selection",
    response: response({ parsed: { selected: [[1, score]] } }),
  });
for (const [index, database, selected] of [
  [0, String.raw`\\?\C:\db\.`, ["db"]],
  [1, String.raw`\\?\C:\dir\foo/bar`, ["bar"]],
  [2, String.raw`\\?\C:\dir\foo/bar`, [String.raw`\\?\C:\foo/bar`]],
  [3, String.raw`\\?\C:\db`, ["db"]],
  [4, String.raw`\\?\UNC\server\share\db`, ["db"]],
  [5, String.raw`\\?\C:\db\..`, ["db"]],
])
  add(`verbatim-${index}`, { op: "database", database, selected });
for (const [index, raw] of [
  "{}",
  "[]",
  "null",
  '"text"',
  "",
  " ",
  "{",
  '{"notifiable_article_ids":[]}',
  '{"notifiable_article_ids":null}',
  '{"notifiable_article_ids":"1"}',
  '{"notifiable_article_ids":[3,1,3,"+2","03",2.0,-0,0,"-0",true,null,9223372036854775807,9223372036854775808]}',
  '{"db_name":" other.sqlite ","notifiable_article_ids":[]}',
  '{"db_name":false,"notifiable_article_ids":[]}',
  '{"db_name":null,"db_name":null,"notifiable_article_ids":[]}',
  '{"run_id":5,"notifiable_article_ids":[]}',
  '{"run_id":"  source  ","notifiable_article_ids":[]}',
  '{"run_id":"  ","notifiable_article_ids":[]}',
  '{"changed_issue_keys":["2:3","1:02","1:2","1:02","bad","2:3",1,"1:2:3","-1:+2"],"changed_inpress_journal_ids":["+2",2,"01",-2,1.0],"notifiable_article_ids":[]}',
  '{"changed_issue_keys":null,"changed_inpress_journal_ids":{},"notifiable_article_ids":[]}',
  '{"unknown":1e9999,"notifiable_article_ids":[]}',
  '{"unknown":"\\ud800","notifiable_article_ids":[]}',
  '{"changed_issue_keys":[1e9999],"notifiable_article_ids":[]}',
  '{"changed_issue_keys":["\\ud800"],"notifiable_article_ids":[]}',
  '\v\f{ "notifiable_article_ids":[]}',
].entries())
  add(`manifest-${index}`, { op: "manifest", raw, database: "fixture.sqlite" });
for (const [index, raw] of [
  "{}",
  "[]",
  "[{}]",
  "[{},{}]",
  "[{},{},{}]",
  "null",
  '{"issue_article_counts":null}',
  '{"issue_article_counts":{"a":3,"b":-2},"inpress_article_counts":{"x":9223372036854775807}}',
  '{"issue_article_counts":{"a":1.0}}',
  '{"issue_article_counts":{"a":"1"}}',
  '{"issue_article_counts":{"a":false,"a":1}}',
  '{"issue_article_counts":{"a":1,"a":2}}',
  '{"issue_article_counts":{},"issue_article_counts":{}}',
  '{"unknown":1e9999}',
  '{"unknown":"\\ud800"}',
  '{"issue_article_counts":{"\\ud800":1}}',
].entries())
  add(`checkpoint-${index}`, { op: "checkpoint", raw });
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/delivery-oracle/recommend-build.json",
    "utf8",
  ),
);
assert.equal(digest(await fs.readFile(build.binary)), build.binary_sha256);
const run = spawnSync(build.binary, [], {
  input: cases.map((item) => item.input).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 30000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(run.error);
assert.equal(run.status, 0, run.stderr);
const lines = run.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, cases.length);
const vectors = cases.map((item, index) => ({ ...item, output: lines[index] }));
await fs.writeFile(
  "tests/migration/delivery/recommend-vectors.json",
  JSON.stringify(
    {
      schema: 1,
      platform: process.platform,
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/delivery/export-recommend.mjs"),
      ),
      cases: vectors,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${vectors.length} original recommendation observations`);
