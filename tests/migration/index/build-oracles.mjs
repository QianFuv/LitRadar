/** Build independent indexing observers against the unchanged locked Rust implementation. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest, WORKSPACE_ROOT } from "../oracle.mjs";

/** Execute a bounded compiler without letting the Go C compiler override Rust's native linker. */
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
  "litradar-index",
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
  "litradar_index",
  "litradar_domain",
  "serde_json",
  "rusqlite",
  "sha2",
  "serde",
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
await fs.mkdir("output/migration/execution/index-oracle", { recursive: true });
const liveSource = await fs.readFile(
  "crates/litradar-index/src/live.rs",
  "utf8",
);
const notifyStart = liveSource.indexOf(
  "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nstruct NotifyHandoffObservation",
);
const notifyEnd = liveSource.indexOf(
  "/// Live index workflow failure.",
  notifyStart,
);
const functionsStart = liveSource.indexOf("fn read_bounded_notify_output(");
const functionsEnd = liveSource.indexOf("#[cfg(test)]", functionsStart);
assert(
  notifyStart > 0 &&
    notifyEnd > notifyStart &&
    functionsStart > 0 &&
    functionsEnd > functionsStart,
);
const notifyOriginal = [
  liveSource.match(/const MAX_NOTIFY_HANDOFF_STDOUT_BYTES:[^\n]+/)[0],
  liveSource.match(/const NOTIFY_HANDOFF_PROTOCOL_VERSION:[^\n]+/)[0],
  liveSource.slice(notifyStart, notifyEnd),
  liveSource.slice(functionsStart, functionsEnd),
].join("\n");
await fs.writeFile(
  "output/migration/execution/index-oracle/notify-original.rs",
  notifyOriginal,
);
const source = "tests/migration/index/identity-oracle.rs";
const binary = `output/migration/execution/index-oracle/identity${process.platform === "win32" ? ".exe" : ""}`;
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
  "crates/litradar-index/src/identity.rs",
  "crates/litradar-index/src/transforms.rs",
  "crates/litradar-index/src/schema.rs",
  "crates/litradar-index/src/control.rs",
  "crates/litradar-index/src/changes.rs",
  "crates/litradar-index/src/batch.rs",
  "tests/migration/index/batch-observer.rs",
  "crates/litradar-index/src/worker_protocol.rs",
  "tests/migration/index/worker-observer.rs",
  "tests/migration/index/notify-observer.rs",
  "crates/litradar-index/src/live.rs",
  "output/migration/execution/index-oracle/notify-original.rs",
  "crates/litradar-sources/src/scholarly.rs",
  "tests/migration/index/control-observer.rs",
  "crates/litradar-storage/src/index_schema.rs",
  "crates/litradar-domain/src/ids.rs",
  "crates/litradar-domain/src/index_contract.rs",
  source,
])
  inputs.push({ path, sha256: digest(await fs.readFile(path)) });
await fs.writeFile(
  "output/migration/execution/index-oracle/identity-build.json",
  JSON.stringify(
    {
      binary,
      binary_sha256: digest(await fs.readFile(binary)),
      builder_sha256: digest(
        await fs.readFile("tests/migration/index/build-oracles.mjs"),
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
console.log("Original Rust indexing identity/catalog observer built");
