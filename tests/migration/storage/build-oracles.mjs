/** Build standalone observations using the exact dependency artifacts selected by locked Cargo resolution. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Run one bounded compiler command and retain its diagnostics on failure. */
function run(command, args) {
  const environment = { ...process.env };
  if (process.platform === "win32") {
    delete environment.CC;
    delete environment.CXX;
  }
  const result = spawnSync(command, args, {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 180000,
    maxBuffer: 32 * 1024 * 1024,
    env: environment,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr + result.stdout);
  return result;
}
const allowed = [
  "citation",
  "index",
  "search",
  "metadata",
  "weekly",
  "weekly-query",
  "favorites",
  "backup",
  "database",
];
const selected = process.argv.slice(2);
if (selected.length === 0) selected.push(...allowed);
for (const name of selected)
  assert(allowed.includes(name), `Unknown oracle ${name}`);
const built = run("cargo", [
  "build",
  "-p",
  "litradar-storage",
  "--locked",
  "--message-format=json",
]);
const artifacts = built.stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line))
  .filter((item) => item.reason === "compiler-artifact" && !item.profile.test);
const externs = [];
const dependencies = [];
for (const name of [
  "litradar_storage",
  "litradar_domain",
  "serde_json",
  "serde",
  "rusqlite",
  "chrono",
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
for (const name of selected) {
  const copiedSources = [];
  if (name === "weekly-query") {
    const destination = "output/migration/execution/weekly-query-index";
    await fs.mkdir(destination, { recursive: true });
    for (const file of [
      "mod.rs",
      "articles.rs",
      "fulltext.rs",
      "metadata.rs",
      "shared.rs",
      "weekly.rs",
    ]) {
      const sourcePath = `crates/litradar-storage/src/index/${file}`;
      const original = await fs.readFile(sourcePath, "utf8");
      let exposed = original;
      if (file === "weekly.rs")
        for (const method of [
          "get_weekly_updates_at",
          "get_weekly_updates_summary_at",
        ]) {
          const needle = `\nfn ${method}(`;
          assert.equal(original.split(needle).length, 2);
          exposed = exposed.replace(needle, `\npub fn ${method}(`);
        }
      if (file === "mod.rs")
        exposed +=
          "\npub use weekly::{get_weekly_updates_at,get_weekly_updates_summary_at};\n";
      await fs.writeFile(`${destination}/${file}`, exposed);
      copiedSources.push({
        path: sourcePath,
        original_sha256: digest(Buffer.from(original)),
        exposed_sha256: digest(Buffer.from(exposed)),
      });
    }
  }
  const source = `tests/migration/storage/${name}-oracle.rs`;
  const binary = `output/migration/execution/${name}-oracle.exe`;
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
  await fs.writeFile(
    `output/migration/execution/t04-${name}-oracle-build.json`,
    JSON.stringify(
      {
        command: "rustc",
        args,
        source_sha256: digest(await fs.readFile(source)),
        binary_sha256: digest(await fs.readFile(binary)),
        dependencies,
        copiedSources,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`Built original Rust ${name} oracle`);
}
