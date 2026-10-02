/**
 * Explicitly prepare the Rust oracle once; normal migration tests never rebuild expected answers.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {
  BASELINE,
  WORKSPACE_ROOT,
  digest,
  oracleDirectory,
} from "./oracle.mjs";

/**
 * Run one bounded fixture/build command and fail before accepting incomplete output.
 * @param {string} command - Executable.
 * @param {string[]} args - Literal arguments.
 * @returns {string} Captured stdout.
 */
function execute(command, args) {
  const result = spawnSync(command, args, {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    shell: false,
    timeout: 600_000,
    maxBuffer: 32 * 1024 * 1024,
    env: { ...process.env, RUST_LOG: "error" },
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} failed: ${result.stderr}`);
  return result.stdout;
}

/**
 * Enumerate regular artifacts recursively while rejecting links.
 * @param {string} directory - Owned archive root.
 * @param {string} [relative=''] - Relative subdirectory.
 * @returns {Promise<object[]>} Exact artifact identities.
 */
async function artifactManifest(directory, relative = "") {
  const entries = [];
  for (const child of await fs.readdir(path.join(directory, relative), {
    withFileTypes: true,
  })) {
    const name = path.posix.join(relative, child.name);
    assert(!child.isSymbolicLink(), "Oracle may not contain symbolic links");
    if (child.isDirectory())
      entries.push(...(await artifactManifest(directory, name)));
    else {
      assert(child.isFile(), "Unexpected oracle artifact type");
      const bytes = await fs.readFile(path.join(directory, name));
      entries.push({ path: name, bytes: bytes.length, sha256: digest(bytes) });
    }
  }
  return entries.sort((left, right) =>
    left.path.localeCompare(right.path, "en"),
  );
}

assert.deepEqual(
  process.argv.slice(2),
  ["--baseline", BASELINE],
  `Usage: node tests/migration/prepare-baseline.mjs --baseline ${BASELINE}`,
);
assert.equal(
  execute("git", ["rev-parse", "HEAD"]).trim(),
  BASELINE,
  "Prepare before the first implementation commit; do not reconstruct an oracle from migrated code",
);
assert(
  ["win32", "linux"].includes(process.platform) && process.arch === "x64",
  "This preparation route supports only native x64; other targets require their own verified native assets",
);
assert.equal(
  execute("git", [
    "diff",
    BASELINE,
    "--",
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "libs/simple",
    "tests/data/scenarios/api",
    "app/lib/generated/openapi.json",
  ]).trim(),
  "",
  "Baseline application or native sources changed",
);
const untracked = execute("git", [
  "ls-files",
  "--others",
  "--exclude-standard",
  "crates",
])
  .trim()
  .split(/\r?\n/)
  .filter(Boolean);
assert(
  untracked.every(
    (filename) => filename === "crates/litradar/examples/migration_fixture.rs",
  ),
  "Unexpected untracked Rust build input",
);
const finalDestination = oracleDirectory(BASELINE);
const identityDirectory = path.join(WORKSPACE_ROOT, "tests/data/migration");
const identityPath = path.join(
  identityDirectory,
  `oracle-${process.platform}-${process.arch}.json`,
);
await assert.rejects(
  fs.lstat(identityPath),
  { code: "ENOENT" },
  "Frozen identity already exists",
);
await assert.rejects(
  fs.lstat(finalDestination),
  { code: "ENOENT" },
  "Oracle already exists",
);
await fs.mkdir(path.dirname(finalDestination), { recursive: true });
const destination = await fs.mkdtemp(`${finalDestination}.preparing-`);
const temporary = await fs.mkdtemp(
  path.join(os.tmpdir(), "litradar-migration-oracle-"),
);
try {
  await fs.writeFile(
    path.join(temporary, ".litradar-migration-fixture"),
    "litradar-migration-fixture-v1\n",
  );
  const buildArguments = [
    "build",
    "--locked",
    "-p",
    "litradar",
    "--bin",
    "litradar",
    "--example",
    "migration_fixture",
    "--example",
    "full_stack_fixture",
    "--message-format=json",
  ];
  const buildMessages = execute("cargo", buildArguments)
    .trim()
    .split(/\r?\n/)
    .map((line) => JSON.parse(line));
  const suffix = process.platform === "win32" ? ".exe" : "";
  const binaries = {
    application: "litradar",
    exporter: "migration_fixture",
    fullStackFixture: "full_stack_fixture",
  };
  for (const [role, target] of Object.entries(binaries)) {
    const artifacts = buildMessages.filter(
      (entry) =>
        entry.reason === "compiler-artifact" &&
        entry.target.name === target &&
        entry.executable,
    );
    assert.equal(
      artifacts.length,
      1,
      `Expected one Cargo executable for ${target}`,
    );
    const name = `${role}${suffix}`;
    await fs.copyFile(artifacts[0].executable, path.join(destination, name));
    binaries[role] = name;
  }
  const nativeName =
    process.platform === "win32" ? "simple.dll" : "libsimple.so";
  const native = `libs/simple/${process.platform === "win32" ? "windows" : "linux"}/${nativeName}`;
  if (process.platform === "linux") {
    await assert.rejects(
      fs.lstat("/usr/lib/litradar/libsimple.so"),
      { code: "ENOENT" },
      "System Simple takes precedence; use a separately documented system-asset oracle route",
    );
  }
  await fs.copyFile(
    path.join(WORKSPACE_ROOT, native),
    path.join(destination, nativeName),
  );
  execute(path.join(destination, binaries.exporter), [temporary]);
  await fs.cp(
    path.join(temporary, "fixtures"),
    path.join(destination, "fixtures"),
    { recursive: true, errorOnExist: true },
  );
  await fs.cp(
    path.join(WORKSPACE_ROOT, "tests/data/scenarios/api"),
    path.join(destination, "shared-api-scenarios"),
    { recursive: true },
  );
  const sources = execute("git", [
    "ls-files",
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "libs/simple",
    "tests/data/scenarios/api",
    "app/lib/generated/openapi.json",
  ])
    .trim()
    .split(/\r?\n/);
  const sourceHashes = [];
  for (const filename of sources) {
    sourceHashes.push({
      path: filename,
      sha256: digest(await fs.readFile(path.join(WORKSPACE_ROOT, filename))),
    });
  }
  const exporter = "crates/litradar/examples/migration_fixture.rs";
  sourceHashes.push({
    path: exporter,
    sha256: digest(await fs.readFile(path.join(WORKSPACE_ROOT, exporter))),
    role: "Approved T01 test-only exporter added to the unchanged application baseline",
  });
  const manifest = {
    format: 1,
    baseline: BASELINE,
    platform: process.platform,
    architecture: process.arch,
    createdAt: new Date().toISOString(),
    syntheticOnly: true,
    binaries,
    buildArguments,
    rustc: execute("rustc", ["-Vv"]).trim(),
    cargo: execute("cargo", ["--version"]).trim(),
    node: process.version,
    nativeSource: native,
    sourceHashes,
    files: await artifactManifest(destination),
  };
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  await fs.writeFile(path.join(destination, "manifest.json"), manifestBytes);
  for (let attempt = 0; ; attempt += 1) {
    try {
      await fs.rename(destination, finalDestination);
      break;
    } catch (error) {
      if (
        process.platform !== "win32" ||
        !["EPERM", "EBUSY"].includes(error.code) ||
        attempt >= 4
      )
        throw error;
      await assert.rejects(fs.lstat(finalDestination), { code: "ENOENT" });
      await delay(250 * (attempt + 1));
    }
  }
  await fs.mkdir(identityDirectory, { recursive: true });
  await fs.writeFile(
    identityPath,
    `${JSON.stringify(
      {
        baseline: BASELINE,
        manifestSha256: digest(manifestBytes),
        platform: process.platform,
        architecture: process.arch,
      },
      null,
      2,
    )}\n`,
    { flag: "wx" },
  );
  process.stdout.write(
    `${JSON.stringify({ directory: finalDestination, artifacts: manifest.files.length })}\n`,
  );
} finally {
  assert(
    path.isAbsolute(temporary) &&
      path.dirname(temporary) === path.resolve(os.tmpdir()) &&
      path.basename(temporary).startsWith("litradar-migration-oracle-"),
    "Unsafe temporary cleanup path",
  );
  await fs.rm(temporary, { recursive: true, force: true });
}
