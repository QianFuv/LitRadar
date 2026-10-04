/** Compile the original storage-transition observer against locked storage dependencies. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest, WORKSPACE_ROOT } from "../oracle.mjs";

/** Run a bounded compiler while preserving the Rust native linker configuration. */
function run(command, args) {
  const env = { ...process.env };
  if (process.platform === "win32") {
    delete env.CC;
    delete env.CXX;
  }
  const result = spawnSync(command, args, {
    cwd: WORKSPACE_ROOT,
    env,
    encoding: "utf8",
    windowsHide: true,
    timeout: 180000,
    maxBuffer: 32 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stdout + result.stderr);
  return result;
}
const built = run("cargo", [
  "build",
  "-p",
  "litradar-storage",
  "-p",
  "litradar-sources",
  "--locked",
  "--message-format=json",
]);
const artifacts = built.stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line))
  .filter((item) => item.reason === "compiler-artifact" && !item.profile.test);
const externs = [],
  dependencies = [];
for (const name of [
  "litradar_storage",
  "litradar_domain",
  "rusqlite",
  "serde_json",
  "chrono",
  "litradar_sources",
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
  assert.equal(files.length, 1, name);
  externs.push("--extern", `${name}=${files[0]}`);
  dependencies.push({
    name,
    path: files[0],
    sha256: digest(await fs.readFile(files[0])),
  });
}
await fs.mkdir("output/migration/execution/cfp-oracle", {
  recursive: true,
});
const source = "tests/migration/cfp/storage-oracle.rs",
  binary = `output/migration/execution/cfp-oracle/storage${process.platform === "win32" ? ".exe" : ""}`;
const args = [
  "--edition=2024",
  source,
  "-L",
  "dependency=target/debug/deps",
  ...externs,
  "-o",
  binary,
];
run("rustc", args);
const inputs = [];
for (const path of [
  "Cargo.toml",
  "Cargo.lock",
  "crates/litradar-storage/Cargo.toml",
  "crates/litradar-storage/src/business/cfp.rs",
  "crates/litradar-domain/src/cfp.rs",
  "crates/litradar-domain/src/cfp/dates.rs",
  "crates/litradar-sources/src/cfp/mod.rs",
  "crates/litradar-sources/src/cfp/html.rs",
  "crates/litradar-sources/src/cfp/http.rs",
  "crates/litradar-storage/src/business/security_audit.rs",
  "crates/litradar-domain/src/business.rs",
  "crates/litradar-storage/src/migrations.rs",
  "crates/litradar-storage/src/config.rs",
  source,
])
  inputs.push({ path, sha256: digest(await fs.readFile(path)) });
await fs.writeFile(
  "output/migration/execution/cfp-oracle/storage-build.json",
  JSON.stringify(
    {
      binary,
      binary_sha256: digest(await fs.readFile(binary)),
      builder_sha256: digest(
        await fs.readFile("tests/migration/cfp/build-storage.mjs"),
      ),
      command: "rustc",
      args,
      inputs,
      dependencies,
    },
    null,
    2,
  ) + "\n",
);
