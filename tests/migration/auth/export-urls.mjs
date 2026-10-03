/** Freeze original Rust responses to the pinned URL dependency's IDNA and WPT input corpora. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const environment = spawnSync("go", ["env", "GOMODCACHE"], {
  encoding: "utf8",
  shell: false,
  windowsHide: true,
  timeout: 60_000,
});
assert.ifError(environment.error);
assert.equal(environment.status, 0, environment.stderr);
const root = path.join(
  environment.stdout.trim(),
  "github.com/nlnwa/whatwg-url@v0.6.2/testdata",
);
const cases = [],
  corpusSources = [];
for (const filename of ["IdnaTestV2.json", "urltestdata.json"]) {
  const data = await fs.readFile(path.join(root, filename));
  corpusSources.push({
    module: "github.com/nlnwa/whatwg-url",
    version: "v0.6.2",
    path: `testdata/${filename}`,
    sha256: digest(data),
  });
  for (const value of JSON.parse(data).filter(
    (value) =>
      value && typeof value === "object" && typeof value.input === "string",
  )) {
    if (filename === "IdnaTestV2.json")
      cases.push({
        field: "ai_allowed_base_urls",
        input: `https://${value.input}`,
      });
    else
      for (const field of ["ai_allowed_base_urls", "provider_proxy_url"])
        cases.push({ field, input: value.input });
  }
}
const valid = cases.filter((value) => value.input.isWellFormed());
const result = spawnSync("output/migration/execution/settings-oracle.exe", [], {
  cwd: WORKSPACE_ROOT,
  input: valid.map((value) => JSON.stringify(value)).join("\n") + "\n",
  encoding: "utf8",
  shell: false,
  windowsHide: true,
  timeout: 60_000,
  maxBuffer: 32 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, valid.length);
assert.equal(valid.length, 3670);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/business/runtime_settings.rs",
  "tests/migration/auth/settings-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/auth/url-vectors.json",
  JSON.stringify(
    {
      baseline: BASELINE,
      sources,
      corpusSources,
      excludedNonScalarStrings: cases.length - valid.length,
      cases: observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Exported ${observations.length} original Rust URL observations`);
