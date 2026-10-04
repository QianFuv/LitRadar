/** Freeze original provider request sequences using synthetic responses only. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const cases = [];
const config = {
  base_url: "https://example.test/v1/",
  api_key: "synthetic-key",
  model: "model",
  system_prompt: "",
};
const candidate = {
  article_id: 7,
  journal_id: 1,
  issue_id: 2,
  title: "A <paper>",
  abstract_text: "😀".repeat(1201),
  date: "2026-10-04",
  journal_title: "Journal",
  doi: null,
  open_access: true,
  in_press: false,
};
const subscriber = {
  subscriber_id: "user-1",
  name: "Reader",
  keywords: ["networks"],
  directions: ["systems"],
};
const message = {
  token: "synthetic-token",
  title: "title",
  content: "content",
  channel: "wechat",
  template: "markdown",
  topic: " group ",
  option: " ",
  to: " reader ",
};
const selected = {
  choices: [
    {
      message: {
        parsed: {
          summary: " model summary ",
          selected: [
            { article_id: 7, score: 1 },
            { article_id: "+9", score: 3 },
          ],
        },
      },
    },
  ],
};
const summarized = {
  choices: [{ message: { parsed: { summary: " final summary " } } }],
};
/** Register an AI input with complete request parameters. */
function ai(name, responses, overrides = {}) {
  cases.push({
    name,
    input: JSON.stringify({
      op: "selection",
      config,
      subscriber,
      candidates: [candidate],
      responses,
      retries: 2,
      ...overrides,
    }),
  });
}
/** Register a PushPlus input with complete request parameters. */
function push(name, responses, overrides = {}) {
  cases.push({
    name,
    input: JSON.stringify({
      op: "pushplus",
      message,
      responses,
      retries: 2,
      ...overrides,
    }),
  });
}
ai("selection-payload", [{ body: selected }]);
ai("summary-payload", [{ body: summarized }], { op: "summary" });
ai("summary-empty-skips", [], { op: "summary", candidates: [] });
ai("empty-candidates", [{ body: selected }], {
  candidates: [],
  subscriber: {},
});
for (const op of ["selection", "summary"]) {
  ai(
    `${op}-custom-prompt`,
    [{ body: op === "selection" ? selected : summarized }],
    { op, config: { ...config, system_prompt: "  custom prompt  " } },
  );
  for (const base of [
    "https://api.deepseek.com/v1/",
    "https://API.DEEPSEEK.COM/v1",
    "https://example.test/v1",
    "https://example.test/%GG/",
    "not a URL",
  ]) {
    ai(
      `${op}-url-${base}`,
      [
        { body: null },
        { body: null },
        { body: op === "selection" ? selected : summarized },
      ],
      { op, config: { ...config, base_url: base } },
    );
  }
}
for (const status of [
  200, 201, 204, 299, 301, 302, 400, 401, 404, 408, 422, 429, 500, 502, 503,
  504,
]) {
  ai(`ai-http-${status}`, [
    { status, body: selected, request_id: "safe:1", retry_after: 0 },
    { body: selected },
  ]);
  push(`push-http-${status}`, [
    {
      status,
      body: { code: 200, data: "id" },
      request_id: "safe:1",
      retry_after: 0,
    },
    { body: { code: 200 } },
  ]);
}
for (const error of ["connect_failed", "timeout", "transport"]) {
  ai(`ai-${error}`, [{ error }, { body: selected }]);
  push(`push-${error}`, [{ error }, { body: { code: 200, data: "ok" } }]);
}
ai("three-format-exhaustion", [{ body: null }, { body: null }, { body: null }]);
ai("format-fallback-does-not-retry", [{ body: null }, { body: selected }]);
ai("retry-then-format", [
  { status: 503, retry_after: 0 },
  { body: null },
  { body: selected },
]);
ai(
  "bounded-eleven-attempts",
  Array.from({ length: 11 }, () => ({ error: "timeout" })),
  { retries: 100 },
);
push(
  "push-bounded-eleven-attempts",
  Array.from({ length: 11 }, () => ({ error: "connect_failed" })),
  { retries: 100 },
);
for (const code of [
  200,
  "200",
  "+200",
  "0200",
  "200.0",
  " 200 ",
  null,
  0,
  500,
  true,
  [],
  {},
])
  push(`push-code-${JSON.stringify(code)}`, [{ body: { code, data: "id" } }]);
cases.push({
  name: "push-floating-code",
  input: JSON.stringify({
    op: "pushplus",
    message,
    retries: 2,
    responses: [{ body: { code: "FLOAT_SENTINEL", data: "id" } }],
  }).replace('"FLOAT_SENTINEL"', "200.0"),
});
for (const data of [null, "", " raw ", 123, true, [1, "x"], { z: 2, a: 1 }])
  push(`push-data-${JSON.stringify(data)}`, [{ body: { code: 200, data } }]);
push("push-empty-body", [{ body: null }]);
push("push-empty-option-omitted", [{ body: { code: 200 } }], {
  message: { ...message, topic: null, to: "", option: "\u2003" },
});

const provenance = JSON.parse(
  await fs.readFile(
    "output/migration/execution/delivery-oracle/client-build.json",
    "utf8",
  ),
);
assert.equal(
  digest(await fs.readFile(provenance.binary)),
  provenance.binary_sha256,
);
assert.equal(
  digest(await fs.readFile("tests/migration/delivery/build-clients.mjs")),
  provenance.builder_sha256,
);
for (const item of [...provenance.inputs, ...provenance.dependencies])
  assert.equal(digest(await fs.readFile(item.path)), item.sha256, item.path);
const observed = spawnSync(provenance.binary, [], {
  input: cases.map((item) => item.input).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(observed.error);
assert.equal(observed.status, 0, observed.stderr);
const outputs = observed.stdout.trimEnd().split(/\r?\n/);
assert.equal(outputs.length, cases.length);
cases.forEach((item, index) => {
  item.output = outputs[index];
  JSON.parse(item.output);
});
await fs.writeFile(
  "tests/migration/delivery/client-vectors.json",
  JSON.stringify(
    {
      provenance,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/delivery/export-clients.mjs"),
      ),
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Exported ${cases.length} original client observations`);
