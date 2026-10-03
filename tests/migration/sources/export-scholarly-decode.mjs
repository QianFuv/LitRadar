/** Freeze serde struct defaults, sequence forms and strict credential configuration. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const requests = [];
const fixture = {
  crossref_status: null,
  crossref_works: [],
  crossref_work_pages: [],
  openalex_source_by_issns: null,
  openalex_source_by_title: null,
  openalex_source_works: [],
  openalex_source_work_pages: [],
  openalex_source_works_plan_restricted: false,
  openalex_source_works_plan_restricted_after_page: null,
  openalex_source_works_status: null,
  openalex_by_doi: {},
  semantic_scholar_status: null,
  semantic_scholar_error: null,
  semantic_scholar_by_doi: {},
};
const config = {
  timeout_seconds: 1,
  openalex_api_keys: [],
  semantic_scholar_api_keys: [],
  crossref_mailtos: [],
  semantic_scholar_worker_id: 0,
  semantic_scholar_process_count: 1,
  semantic_scholar_base_interval_ms: 1100,
  schedule_epoch_unix_millis: 0,
};
for (const [kind, base] of [
  ["fixture_decode", fixture],
  ["config_decode", config],
]) {
  const inputs = new Set([
    "null",
    "{}",
    "[]",
    JSON.stringify(base),
    JSON.stringify(Object.values(base)),
  ]);
  for (const key of Object.keys(base)) {
    for (const value of [null, 0, -1, 1.5, "", true, [], {}, [null], [[]]])
      inputs.add(JSON.stringify({ ...base, [key]: value }));
    const missing = { ...base };
    delete missing[key];
    inputs.add(JSON.stringify(missing));
    inputs.add(
      JSON.stringify(base).slice(0, -1) +
        `,"${key}":${JSON.stringify(base[key])}}`,
    );
  }
  for (let length = 0; length <= Object.keys(base).length + 1; length++)
    inputs.add(JSON.stringify([...Object.values(base), null].slice(0, length)));
  for (const tail of [
    '"unknown":1e400',
    '"unknown":"\\uD800"',
    '"unknown":null,"unknown":true',
  ])
    inputs.add(JSON.stringify(base).slice(0, -1) + "," + tail + "}");
  const numeric =
    kind === "fixture_decode" ? "crossref_status" : "timeout_seconds";
  for (const value of [
    "-0",
    "0.0",
    "1e0",
    "65535",
    "65536",
    "18446744073709551615",
    "18446744073709551616",
  ])
    inputs.add(
      JSON.stringify(base).replace(
        `"${numeric}":${JSON.stringify(base[numeric])}`,
        `"${numeric}":${value}`,
      ),
    );
  for (const input of inputs) requests.push({ kind, input });
}
const result = spawnSync(
  "output/migration/execution/sources-transport-oracle.exe",
  [],
  {
    input: requests.map((item) => JSON.stringify(item)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  },
);
assert.equal(result.status, 0, result.stderr || String(result.error));
assert.ifError(result.error);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, requests.length);
await fs.writeFile(
  "tests/migration/sources/scholarly-decode-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-transport-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile(
          "tests/migration/sources/export-scholarly-decode.mjs",
        ),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust scholarly decode observations`,
);
