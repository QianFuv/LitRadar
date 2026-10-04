/** Freeze original helper envelopes, cached recovery and durable worker outcomes. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const config = {
  sourceKey: "journal:a",
  catalogIds: ["a"],
  journalTitle: "Example Journal",
  discoveryUrl: "https://example.org/calls",
  adapter: "keai_calls",
  configVersion: 1,
  allowedUrls: [{ host: "example.org", pathPrefix: "/" }],
  identityTexts: ["Example Journal"],
  emptyStatements: ["No open calls for papers"],
  capabilityNote: null,
  retainsPreviousNotices: false,
};
const source = {
  catalogIds: ["a"],
  journalTitle: "Example Journal",
  title: "Original topic",
  typeText: "Special Issue",
  dateText: "Submission deadline: 31 December 2026",
  sourceUrl: config.discoveryUrl,
  scope: "Original preview...",
  checkedOn: "2026-09-15",
};
const complete =
  "<h1 data-test='collection-title'>Original topic</h1><div data-test='collection-description'><p>Every original research topic.</p><p>Authors should prepare the complete experimental protocol.</p></div>";
const list =
  "<h1>Example Journal</h1><h2><a href='/collections/original'>Original topic</a></h2><p>Original preview...</p>";
const documents = [
  { url: source.sourceUrl, text: list },
  { url: "https://example.org/collections/original", text: complete },
];
const cases = [];
/** Add one observable call to the original worker. */
function add(name, input) {
  cases.push({ name, input: { config, ...input } });
}
const envelope = {
  protocol: "litradar.cfp.page.v1",
  finalUrl: config.discoveryUrl,
  html: "<main>Original</main>",
};
for (const payload of [
  JSON.stringify(envelope),
  JSON.stringify(Object.values(envelope)),
  "null",
  "{}",
  "[]",
  "{}{}",
  "log\n{}",
  ...Object.keys(envelope).flatMap((key) => [
    JSON.stringify({ ...envelope, [key]: null }),
    JSON.stringify({ ...envelope, [key]: 1 }),
  ]),
  JSON.stringify({ ...envelope, extra: 1 }),
  JSON.stringify(envelope).replace('"html":', '"protocol":"duplicate","html":'),
  JSON.stringify({ ...envelope, html: " \n" }),
  JSON.stringify({ ...envelope, finalUrl: "https://foreign.example/calls" }),
  JSON.stringify({ ...envelope, protocol: "wrong" }),
  ...Object.values(envelope).map((_, index) =>
    JSON.stringify(
      Object.values(envelope).map((value, position) =>
        position === index ? null : value,
      ),
    ),
  ),
])
  add(`envelope-${cases.length}`, { op: "envelope", payload });
add("recover-linked-original", { op: "recover", source, documents });
add("recover-error-cache", {
  op: "recover",
  source,
  documents: [documents[0], { ...documents[1], error: true }],
});
add("recover-expired", { op: "recover", source, documents, expired: true });
add("recover-cancelled-cached-original", {
  op: "recover",
  source,
  documents,
  cancelled: true,
});
for (const [original, title] of [
  ["İ", "i"],
  ["İ", "İ"],
  ["ΟΣ", "οσ"],
  ["ΟΣ", "ος"],
])
  add(`unicode-title-${original}-${title}`, {
    op: "recover",
    source: { ...source, title: original },
    documents: [
      {
        url: source.sourceUrl,
        text: complete.replace("Original topic", title),
      },
    ],
  });
const seed = (sources) =>
  JSON.stringify({ formatVersion: 1, sources, emptyJournals: [] });
const captured = (records) => ({
  result: { sourceKey: config.sourceKey },
  documents: records.map((item) => ({
    requestedUrl: item.url,
    url: item.url,
    text: item.text,
    format: "html",
  })),
  browserAttempts: records.map((item) => item.url),
});
add("full-complete", {
  op: "full",
  seed: seed([source]),
  capture: captured(documents),
});
add("full-partial", {
  op: "full",
  seed: seed([
    source,
    {
      ...source,
      title: "Unresolved topic",
      sourceUrl: "https://example.org/unresolved",
    },
  ]),
  capture: captured([
    ...documents,
    {
      url: "https://example.org/unresolved",
      text: "<h1>Unverified announcement</h1>",
    },
  ]),
});
add("full-zero", {
  op: "full",
  seed: seed([source]),
  capture: captured([
    { url: source.sourceUrl, text: "<h1>Unverified announcement</h1>" },
  ]),
});
add("full-cancelled", {
  op: "full",
  seed: seed([source]),
  capture: captured(documents),
  cancelled: true,
});
add("full-expired", {
  op: "full",
  seed: seed([source]),
  capture: captured(documents),
  expired: true,
});
add("full-no-notices", {
  op: "full",
  seed: seed([]),
  capture: captured([]),
  cancelled: true,
});
add("full-mixed-resume", {
  op: "full",
  seed: seed([source]),
  capture: {
    ...captured(documents),
    documents: [null, 1, {}, ...captured(documents).documents],
  },
});
for (const capture of [
  null,
  [],
  { result: { sourceKey: "journal:wrong" }, documents: [] },
])
  add(`full-identity-${cases.length}`, {
    op: "full",
    seed: seed([source]),
    capture,
  });
const html =
  "<h1>Example Journal</h1><h2>Call for papers</h2><h3>New original announcement</h3><p>New research scope.</p><p>Submission deadline: 1 December 2027</p>";
add("discovery-replace", { op: "refresh", seed: seed([source]), html });
add("discovery-retain", {
  op: "refresh",
  seed: seed([source]),
  html,
  config: { ...config, retainsPreviousNotices: true },
});
add("discovery-challenge", {
  op: "refresh",
  seed: seed([source]),
  html: "<title>Just a moment...</title>",
});
add("discovery-unsupported", {
  op: "refresh",
  seed: seed([source]),
  html,
  config: { ...config, adapter: "snapshot_only" },
});
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/cfp-oracle/worker-build.json",
    "utf8",
  ),
);
const result = spawnSync(build.binary, [], {
  input: cases.map((entry) => JSON.stringify(entry.input)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 120000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const lines = result.stdout.trim().split(/\r?\n/);
assert.equal(lines.length, cases.length);
for (let index = 0; index < cases.length; index++)
  cases[index].expected = JSON.parse(lines[index]);
assert.equal(
  cases.find((entry) => entry.name === "full-partial").expected.outcome.status,
  "partial",
);
assert.equal(
  cases.find((entry) => entry.name === "discovery-retain").expected.outcome
    .notices,
  2,
);
await fs.writeFile(
  "tests/migration/cfp/worker-vectors.json",
  JSON.stringify(
    {
      provenance: build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/cfp/export-worker.mjs"),
      ),
      observedDate: new Date().toISOString().slice(0, 10),
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log({ workerObservations: cases.length });
