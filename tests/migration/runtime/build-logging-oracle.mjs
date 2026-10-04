/** Build the logging observer against the dependency artifacts selected by locked Cargo. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Run a bounded compiler command, preserving diagnostics on failure. */
function run(command, args) {
  const env = { ...process.env };
  if (process.platform === "win32") {
    delete env.CC;
    delete env.CXX;
  }
  const result = spawnSync(command, args, {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 180000,
    maxBuffer: 32 * 1024 * 1024,
    env,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr + result.stdout);
  return result;
}
const built = run("cargo", [
  "build",
  "-p",
  "litradar",
  "--locked",
  "--message-format=json",
]);
const artifacts = built.stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line))
  .filter(
    (item) =>
      item.reason === "compiler-artifact" &&
      !item.profile.test &&
      item.profile.debuginfo === 2,
  );
await fs.writeFile(
  "output/migration/execution/t11-logging-cargo-artifacts.json",
  JSON.stringify(artifacts, null, 2),
);
const externs = [],
  dependencies = [];
for (const name of [
  "matchers",
  "serde_json",
  "tracing",
  "tracing_subscriber",
]) {
  const files = [
    ...new Set(
      artifacts
        .filter((item) => item.target.name === name)
        .flatMap((item) =>
          item.filenames.filter((file) => file.endsWith(".rlib")),
        ),
    ),
  ];
  assert.equal(files.length, 1, `Ambiguous artifact for ${name}`);
  externs.push("--extern", `${name}=${files[0]}`);
  dependencies.push({
    name,
    path: files[0],
    sha256: digest(await fs.readFile(files[0])),
  });
}
const source = "tests/migration/runtime/logging-oracle.rs";
const binary = "output/migration/execution/logging-oracle.exe";
run("rustc", [
  "--edition=2024",
  source,
  "-L",
  "dependency=target/debug/deps",
  ...externs,
  "-o",
  binary,
]);
await fs.writeFile(
  "output/migration/execution/t11-logging-oracle-build.json",
  JSON.stringify(
    {
      source_sha256: digest(await fs.readFile(source)),
      binary_sha256: digest(await fs.readFile(binary)),
      dependencies,
    },
    null,
    2,
  ) + "\n",
);
console.log("Built original locked logging oracle");
