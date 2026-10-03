/** Freeze original backup bytes and independently observed serde manifest boundaries. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Execute the original Rust public backup API with explicit input/output directories. */
function observe(requests) {
  const result = spawnSync("output/migration/execution/backup-oracle.exe", [], {
    cwd: WORKSPACE_ROOT,
    input: requests.map((request) => JSON.stringify(request)).join("\n") + "\n",
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 32 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout
    .trim()
    .split(/\r?\n/)
    .map((line) => JSON.parse(line));
}
const temporary = await fs.mkdtemp(
  path.join(WORKSPACE_ROOT, "output/migration/execution/backup-oracle-"),
);
await fs.mkdir(path.join(temporary, "data/index"), { recursive: true });
await fs.mkdir(path.join(temporary, "data/meta/nested"), { recursive: true });
await fs.mkdir(path.join(temporary, "data/push_state"), { recursive: true });
await fs.copyFile(
  "tests/migration/storage/fixtures/favorites-auth.sqlite.fixture",
  path.join(temporary, "data/auth.sqlite"),
);
await fs.copyFile(
  "tests/migration/storage/fixtures/metadata.sqlite.fixture",
  path.join(temporary, "data/index/metadata.sqlite"),
);
await fs.writeFile(
  path.join(temporary, "data/meta/nested/catalog.csv"),
  "original Rust backup metadata\n",
);
await fs.writeFile(
  path.join(temporary, "data/push_state/publication.json"),
  "{}\n",
);
const output = path.join(temporary, "snapshot");
const [created] = observe([{ operation: "create", root: temporary, output }]);
assert.equal(created.error, null, created.error);
const frozen = "tests/migration/storage/fixtures/backup-v2";
await fs.mkdir(frozen, { recursive: true });
for (const relative of [
  "manifest.json",
  ...created.output.components.map((component) => component.path),
]) {
  const destination = path.join(frozen, relative);
  await fs.mkdir(path.dirname(destination), { recursive: true });
  await fs.copyFile(path.join(output, relative), destination);
}
const manifest = {
  format: "litradar-backup",
  version: 2,
  created_at: 1,
  selection: { metadata: true, index_databases: false, push_state: false },
  components: [],
};
const inputs = [
  JSON.stringify(manifest),
  JSON.stringify({ ...manifest, unknown: { ignored: "value" } }),
  JSON.stringify({
    ...manifest,
    version: 1,
    selection: { index_databases: false, push_state: false },
  }),
  JSON.stringify({ ...manifest, version: 1.0 }),
  JSON.stringify({ ...manifest, created_at: -1 }),
  JSON.stringify({ ...manifest, created_at: 1.5 }),
  `["litradar-backup",2,1,[true,false,false],[]]`,
  `["litradar-backup",2,1,[false,false],[]]`,
];
const raw = JSON.stringify(manifest);
for (const field of [
  "format",
  "version",
  "created_at",
  "selection",
  "components",
]) {
  const missing = { ...manifest };
  delete missing[field];
  inputs.push(
    JSON.stringify(missing),
    JSON.stringify({ ...manifest, [field]: null }),
  );
}
inputs.push(
  raw.replace('"version":2', '"version":2,"version":2'),
  raw.replace('"version":2', '"version":2.0'),
  raw.replace('"version":2', '"version":-0'),
  raw.replace('"version":2', '"version":4294967296'),
  raw.replace('"created_at":1', '"created_at":1e999'),
);
const component = {
  kind: "auth_database",
  path: "auth.sqlite",
  size: 0,
  sha256: "abc",
  schema_version: 20,
};
for (const value of [
  component,
  { ...component, schema_version: null },
  Object.fromEntries(
    Object.entries(component).filter(([key]) => key !== "schema_version"),
  ),
  ["auth_database", "auth.sqlite", 0, "abc", 20],
  ["auth_database", "auth.sqlite", 0, "abc"],
  { ...component, kind: "unknown" },
  { ...component, size: -1 },
  { ...component, kind: { auth_database: null } },
  { ...component, kind: { auth_database: 0 } },
  { ...component, kind: {} },
  { ...component, kind: { auth_database: null, metadata: null } },
])
  inputs.push(JSON.stringify({ ...manifest, components: [value] }));
inputs.push(
  raw.replace('"version":2', '"unknown":{"ignored":"\\ud800"},"version":2'),
  JSON.stringify({ ...manifest, components: [component] }).replace(
    '"schema_version":20',
    '"schema_version":-0',
  ),
  JSON.stringify({ ...manifest, components: [component] }).replace(
    '"schema_version":20',
    '"schema_version":0',
  ),
);
const cases = observe(inputs.map((input) => ({ operation: "parse", input })));
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/backup.rs",
  "tests/migration/storage/backup-oracle.rs",
  "tests/migration/storage/export-backup.mjs",
  `${frozen}/manifest.json`,
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/backup-vectors.json",
  JSON.stringify({ baseline: BASELINE, sources, cases }, null, 2) + "\n",
);
console.log(
  `Exported original v2 backup and ${cases.length} manifest observations`,
);
