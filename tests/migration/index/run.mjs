/** Verify frozen Rust indexing evidence and execute native Windows/Linux recovery checks. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/index/...",
  "./internal/storage/index/...",
  "./internal/domain/index/...",
  "./internal/platform/process",
];
const INPUTS = [
  "go.mod",
  "go.sum",
  "libs/simple",
  "internal",
  "third_party/go-sqlite3",
  "third_party/go-sqlite3-patches",
  "tests/data/migration",
  "tests/migration",
  "crates",
  "Cargo.toml",
  "Cargo.lock",
];
const CORPORA = {
  identity: 574,
  content: 86,
  control: 50,
  manifest: 12,
  batch: 109,
  worker: 284,
  notify: 110,
};

/** Hash every relevant tracked and untracked source file while rejecting links. */
async function identities(paths) {
  const entries = [];
  for (const relative of [...paths].sort()) {
    const filename = path.join(WORKSPACE_ROOT, relative),
      metadata = await fs.lstat(filename);
    assert(!metadata.isSymbolicLink(), `Unexpected source link: ${relative}`);
    if (metadata.isDirectory())
      entries.push(
        ...(await identities(
          (await fs.readdir(filename)).map((name) => `${relative}/${name}`),
        )),
      );
    else
      entries.push({
        path: relative,
        sha256: digest(await fs.readFile(filename)),
      });
  }
  return entries;
}

/** Permit compiler-generated binary identity changes while retaining exact build input identities. */
function compilationInputs(build) {
  const { binary_sha256, ...inputs } = build;
  assert.match(binary_sha256, /^[0-9a-f]{64}$/);
  return inputs;
}

/** Refuse stale exporters, original sources, linked dependencies or generated observer sections. */
async function validateSources() {
  const baseline = await loadOracle(BASELINE);
  const build = JSON.parse(
    await fs.readFile(
      "output/migration/execution/index-oracle/identity-build.json",
      "utf8",
    ),
  );
  assert.equal(
    build.builder_sha256,
    digest(await fs.readFile("tests/migration/index/build-oracles.mjs")),
  );
  assert.equal(build.binary_sha256, digest(await fs.readFile(build.binary)));
  for (const input of [...build.inputs, ...build.dependencies])
    assert.equal(
      input.sha256,
      digest(await fs.readFile(input.path)),
      `Changed oracle input: ${input.path}`,
    );
  for (const [name, count] of Object.entries(CORPORA)) {
    const corpus = JSON.parse(
      await fs.readFile(`tests/migration/index/${name}-vectors.json`, "utf8"),
    );
    assert.equal(
      corpus.observations.length,
      count,
      `Changed observation inventory: ${name}`,
    );
    assert.equal(
      corpus.exporter_sha256,
      digest(await fs.readFile(`tests/migration/index/export-${name}.mjs`)),
      `Stale exporter: ${name}`,
    );
    assert.deepEqual(
      compilationInputs(corpus.build),
      compilationInputs(build),
      `Stale oracle provenance: ${name}`,
    );
  }
  return {
    counts: CORPORA,
    build,
    oracleManifestSha256: digest(
      await fs.readFile(path.join(baseline.directory, "manifest.json")),
    ),
  };
}

/** Require actual crash recovery, pipe pressure and notification checks in both native runtimes. */
async function requireRecoveryEvidence(result, platform) {
  const events = (await fs.readFile(result.log, "utf8"))
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  const required = [
    "TestIndexRuntimeTokenizerIdentity",
    "TestActualKillRestartAtEveryIndexDurableBoundary",
    "TestActualWorkerBackpressurePreservesLargePages",
    "TestActualThreeWorkersDurablyCommitNinePages",
    "TestActualNotificationProcessDrainsAndClassifies",
    "TestActualNotificationCancellationAndSpawnFailure",
    "TestSuccessfulWorkerReceivesStdinEofBeforeWait",
    "TestLiveUnknownNotificationRequiresAcknowledgmentAndNewAttempt",
    "TestLiveFailedPageResumesCommittedCheckpoint",
    "TestLiveRejectsIncompleteCatalogLedgerBeforeAnyExecution",
  ];
  if (platform === "windows")
    required.push(
      "TestOriginalRustGoContentFileHandoff",
      "TestOriginalRustGoControlFileHandoff",
      "TestOriginalRustGoBatchFileHandoff",
    );
  for (const test of required)
    assert(
      events.some((event) => event.Test === test && event.Action === "pass"),
      `Missing ${platform} proof: ${test}`,
    );
  const boundaries = events.filter(
    (event) =>
      event.Action === "pass" &&
      event.Test?.startsWith(
        "TestActualKillRestartAtEveryIndexDurableBoundary/",
      ),
  );
  assert.equal(
    boundaries.length,
    15,
    "Every declared durable crash boundary must execute",
  );
}

/** Execute V06 with explicit toolchain checks, bounded commands and immutable source identity. */
export async function runIndex() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route requires Windows and the approved WSL Linux toolchain",
  );
  const sourceIdentity = await identities(INPUTS);
  const destination = "output/migration/execution/index-result.json";
  const report = {
    phase: "index",
    result: "In Progress",
    baseline: BASELINE,
    tags: "sqlite_fts5",
    sourceIdentity,
    commands: [],
  };
  /** Persist an interrupted check as incomplete instead of retaining an earlier pass. */
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Record operation identity before launch and retain the complete result afterward. */
  const record = async (id, executable, args) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  try {
    const inventory = await record("index-original-inventory", "git", [
      "-c",
      "core.quotePath=false",
      "ls-tree",
      "-r",
      "--name-only",
      "9f305d9b71dc3ea6a739a6598aa762a4533203f0",
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
    ]);
    const expected = (await fs.readFile(inventory.log, "utf8"))
      .trim()
      .split(/\r?\n/)
      .sort();
    const actual = sourceIdentity
      .filter(
        (item) =>
          item.path.startsWith("crates/") ||
          ["Cargo.toml", "Cargo.lock"].includes(item.path),
      )
      .map((item) => item.path)
      .sort();
    assert.deepEqual(
      actual,
      expected,
      "Original Rust source inventory changed",
    );
    await record("index-original-source", "git", [
      "diff",
      "--exit-code",
      BASELINE,
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
      ":(exclude)crates/litradar/examples/migration_fixture.rs",
    ]);
    await record("index-original-fixture", "git", [
      "diff",
      "--exit-code",
      "9f305d9b71dc3ea6a739a6598aa762a4533203f0",
      "--",
      "crates/litradar/examples/migration_fixture.rs",
    ]);
    report.dependency = await verifyDependency("go-sqlite3");
    await record("index-build-oracles", process.execPath, [
      "tests/migration/index/build-oracles.mjs",
    ]);
    report.inputs = await validateSources();
    await save();
    const owned = sourceIdentity
      .filter(
        (item) =>
          /^(internal\/(index|storage\/index|domain\/index|platform\/process)\/)/.test(
            item.path,
          ) && item.path.endsWith(".go"),
      )
      .map((item) => item.path);
    const formatting = await record("index-formatting", "gofmt", [
      "-l",
      ...owned,
    ]);
    assert.equal(
      (await fs.readFile(formatting.log, "utf8")).trim(),
      "",
      "Unformatted Go source",
    );
    for (const platform of ["windows", "linux"]) {
      /** Keep Linux execution within the approved compiler/cache environment and process deadline. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-index-${id}`, "go", args)
          : record(`linux-index-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "360s",
              "env",
              "GOWORK=off",
              "GOTOOLCHAIN=local",
              "GOENV=off",
              "GOFLAGS=",
              "GOMODCACHE=/mnt/c/Users/57676/go/pkg/mod",
              "GOCACHE=/home/qianfuv/.cache/litradar-migration/go-cache",
              "/home/qianfuv/.cache/litradar-migration/go1.27.1/go/bin/go",
              ...args,
            ]);
      await invoke("environment", [
        "env",
        "-json",
        "GOVERSION",
        "GOOS",
        "GOARCH",
        "CGO_ENABLED",
        "CC",
        "GOWORK",
        "GOTOOLCHAIN",
        "GOFLAGS",
        "CGO_CFLAGS",
        "CGO_CPPFLAGS",
        "CGO_CXXFLAGS",
        "CGO_LDFLAGS",
      ]);
      await invoke("build-list", ["list", "-m", "-json", "all"]);
      for (const isRace of [false, true]) {
        const result = await invoke(isRace ? "race" : "regular", [
          "test",
          "-json",
          "-count=1",
          "-mod=readonly",
          "-timeout=180s",
          ...(isRace ? ["-race"] : []),
          "-tags",
          "sqlite_fts5",
          ...PACKAGES,
        ]);
        await requireRecoveryEvidence(result, platform);
      }
    }
    await record("index-module-verify", "go", ["mod", "verify"]);
    await record("index-vet", "go", [
      "vet",
      "-tags",
      "sqlite_fts5",
      ...PACKAGES,
    ]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source inputs changed during V06",
    );
    report.result = "Passed";
    report.limitations = [
      "T06 owns indexing and notification handoff; public CLI/registry wiring remains T11.",
      "Live Rust/Go file handoff executes on Windows; Linux executes identical frozen workflows and native recovery/race checks.",
      "Crash tests force-kill real processes and explicitly expire retained leases before recovery rather than waiting 300 wall-clock seconds.",
      "Finite differential corpora are evidence for their declared inputs, not a proof of every malformed JSON or filesystem path.",
      "Notification subprocesses are synthetic local children; no production messages or provider requests are sent.",
    ];
    await save();
    return {
      result: report.result,
      commands: report.commands.length,
      observations: report.inputs.counts,
    };
  } catch (error) {
    report.result = "Failed";
    report.error = String(error);
    await save();
    throw error;
  }
}
