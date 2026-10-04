/** Compile the original worker-transition observer against locked worker dependencies. */
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
  "litradar-worker",
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
  "chrono",
  "chrono_tz",
  "command_group",
  "serde",
  "serde_json",
  "tracing",
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
await fs.mkdir("output/migration/execution/scheduler-oracle", {
  recursive: true,
});
const source = "tests/migration/scheduler/worker-oracle.rs",
  binary = `output/migration/execution/scheduler-oracle/worker${process.platform === "win32" ? ".exe" : ""}`;
const args = [
  "--edition=2024",
  source,
  "-L",
  "dependency=target/debug/deps",
  ...externs,
  "-o",
  binary,
];
const original = await fs.readFile(
  "crates/litradar-worker/src/scheduler.rs",
  "utf8",
);
const wrapper = await fs.readFile(
  "tests/migration/scheduler/worker-observer.rs",
  "utf8",
);
await fs.writeFile(
  "output/migration/execution/scheduler-oracle/scheduler.rs",
  original + "\n" + wrapper,
);
const main = await fs.readFile(source, "utf8");
await fs.writeFile(
  "output/migration/execution/scheduler-oracle/worker-main.rs",
  '#![allow(dead_code)]\n#[path="../../../../crates/litradar-worker/src/process_supervisor.rs"] mod process_supervisor;\nmod scheduler;\n' +
    main.replace(/^\/\/!.*\r?\n/, ""),
);
args[1] = "output/migration/execution/scheduler-oracle/worker-main.rs";
run("rustc", args);
const inputs = [];
for (const path of [
  "Cargo.toml",
  "Cargo.lock",
  "crates/litradar-worker/Cargo.toml",
  "crates/litradar-worker/src/scheduler.rs",
  "crates/litradar-worker/src/process_supervisor.rs",
  "tests/migration/scheduler/worker-observer.rs",
  "crates/litradar-storage/src/business/scheduled_tasks.rs",
  "crates/litradar-domain/src/business.rs",
  "crates/litradar-storage/src/migrations.rs",
  "crates/litradar-storage/src/config.rs",
  source,
])
  inputs.push({ path, sha256: digest(await fs.readFile(path)) });
await fs.writeFile(
  "output/migration/execution/scheduler-oracle/worker-build.json",
  JSON.stringify(
    {
      binary,
      binary_sha256: digest(await fs.readFile(binary)),
      builder_sha256: digest(
        await fs.readFile("tests/migration/scheduler/build-worker.mjs"),
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
