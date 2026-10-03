/** Build independent original Rust verification and pass current synthetic Go envelopes through stdin. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Execute a bounded compiler or oracle and reject missing or partial results. */
function execute(executable, args, input) {
  const result = spawnSync(executable, args, {
    cwd: WORKSPACE_ROOT,
    shell: false,
    windowsHide: true,
    encoding: "utf8",
    input,
    timeout: 540_000,
    maxBuffer: 32 * 1024 * 1024,
    env: {
      ...process.env,
      GOWORK: "off",
      GOTOOLCHAIN: "go1.27.1",
      GOENV: "off",
      GOFLAGS: "",
    },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

const build = execute("cargo", [
  "build",
  "-p",
  "litradar-storage",
  "--locked",
  "--message-format=json",
]);
const artifacts = build
  .split(/\r?\n/)
  .filter((line) => line.startsWith("{"))
  .map((line) => JSON.parse(line));
/** Select the actual non-test library from Cargo's verified build rather than guessing an artifact hash. */
function library(name) {
  const candidates = artifacts
    .filter(
      (item) =>
        item.reason === "compiler-artifact" &&
        item.target.name === name &&
        !item.profile.test,
    )
    .flatMap((item) => item.filenames)
    .filter((name) => name.endsWith(".rlib"));
  assert.equal(candidates.length, 1, `Ambiguous Rust artifact: ${name}`);
  return candidates[0];
}
const storage = library("litradar_storage"),
  serde = library("serde_json");
const destination = "output/migration/execution/secret-oracle.exe";
execute("rustc", [
  "--edition=2024",
  "tests/migration/auth/secret-oracle.rs",
  "-L",
  "dependency=target/debug/deps",
  "--extern",
  `litradar_storage=${storage}`,
  "--extern",
  `serde_json=${serde}`,
  "-o",
  destination,
]);
const candidate = execute("go", [
  "run",
  "-mod=readonly",
  "-tags",
  "sqlite_fts5",
  "./tests/migration/auth/secret-candidate",
]);
const result = JSON.parse(execute(destination, [], candidate));
assert.equal(result.verified, 28);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/secrets.rs",
  "tests/migration/auth/secret-oracle.rs",
  "tests/migration/auth/secret-candidate/main.go",
  storage,
  serde,
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "output/migration/execution/secret-interop-result.json",
  JSON.stringify(
    {
      result: "Passed",
      ...result,
      sources,
      observationsSha256: digest(candidate),
    },
    null,
    2,
  ) + "\n",
);
console.log(JSON.stringify(result));
