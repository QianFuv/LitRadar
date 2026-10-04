/** Freeze exact original manifest bytes, integer ordering and bounded outbox page semantics. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/index-oracle/identity-build.json",
    "utf8",
  ),
);
const requests = [];
/** Add a provider-independent immutable publication case. */
function scenario(
  name,
  events,
  database = "example.sqlite",
  run = "run-1",
  generated = "1791072000",
) {
  requests.push({ op: "manifest", name, events, database, run, generated });
}
const base = {
  event_id: "1",
  article_id: "9007199254740993",
  journal_id: "10",
  issue_id: "2",
  kind: "upsert",
  in_press: "0",
};
scenario("empty", []);
scenario("single", [base]);
scenario("upsert-and-remove", [
  base,
  { ...base, event_id: "2", kind: "remove" },
]);
scenario("numeric-and-lexical-order", [
  base,
  { ...base, event_id: "2", article_id: "-9", journal_id: "2", issue_id: "1" },
  {
    ...base,
    event_id: "3",
    article_id: "9223372036854775807",
    journal_id: "-1",
    issue_id: "-2",
    kind: "remove",
  },
  {
    ...base,
    event_id: "4",
    article_id: "-9223372036854775808",
    journal_id: "9007199254740993",
    in_press: "2",
  },
]);
scenario("nonpositive-events-ignored", [
  { ...base, event_id: "-1" },
  { ...base, event_id: "0" },
  { ...base, event_id: "8" },
]);
scenario("unknown-kind-still-membership", [
  { ...base, kind: "other" },
  { ...base, event_id: "2", in_press: "-1", kind: "UPSERT" },
]);
scenario("unassociated-article", [{ ...base, issue_id: null }]);
scenario(
  "verbatim-labels",
  [base],
  " <&>\u2028\u2029.sqlite ",
  "run\n\u0000中文",
  " not-a-timestamp ",
);
for (const count of [999, 1000, 1001, 2001])
  scenario(
    `page-${count}`,
    Array.from({ length: count }, (_, position) => ({
      ...base,
      event_id: String(position + 1),
      article_id: String((position % 7) + 1),
      journal_id: String((position % 13) + 1),
      in_press: String(position % 2),
      kind: position % 3 === 0 ? "remove" : "upsert",
    })),
  );
const observations = [];
for (const input of requests) {
  const result = spawnSync(build.binary, [], {
    input: JSON.stringify(input) + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.ifError(result.error);
  observations.push(JSON.parse(result.stdout));
}
assert.equal(
  observations.find((value) => value.input.name === "page-2001").expected.count,
  2001,
);
assert(
  observations
    .find((value) => value.input.name === "single")
    .expected.payload.includes("9007199254740993"),
);
await fs.writeFile(
  "tests/migration/index/manifest-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-manifest.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust manifest observations`,
);
