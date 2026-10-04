/** Verify public Go runtime behavior and frozen original logging/static contracts. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./cmd/litradar/...",
  "./cmd/litradar-fixture/...",
  "./internal/cli/...",
  "./internal/runtime/...",
  "./internal/testkit/fullstack/...",
  "./internal/api/...",
  "./internal/platform/...",
  "./internal/scheduler/...",
  "./internal/delivery/...",
  "./internal/recommend/...",
  "./internal/index/...",
  "./internal/storage/...",
];
const INPUTS = [
  "go.mod",
  "go.sum",
  "cmd",
  "internal",
  "assets",
  "third_party",
  "libs/simple",
  "tests/migration",
  "tests/data/migration",
  "tests/test.mjs",
  "scripts/dev.mjs",
  "scripts/build-go.mjs",
  "app/package.json",
  "app/pnpm-lock.yaml",
  "app/scripts",
  "app/lib",
  "app/tests",
  "crates",
  "Cargo.toml",
  "Cargo.lock",
];

/** Capture relevant tracked and untracked bytes, rejecting links in source inputs. */
async function identities(paths) {
  const result = [];
  for (const relative of [...paths].sort()) {
    const filename = path.join(WORKSPACE_ROOT, relative),
      metadata = await fs.lstat(filename);
    assert(!metadata.isSymbolicLink(), `Unexpected source link: ${relative}`);
    if (metadata.isDirectory())
      result.push(
        ...(await identities(
          (await fs.readdir(filename)).map((name) => `${relative}/${name}`),
        )),
      );
    else
      result.push({
        path: relative,
        sha256: digest(await fs.readFile(filename)),
      });
  }
  return result;
}

/** Require unchanged independent observer sources and exporter identities. */
async function validateCorpora() {
  const oracle = await loadOracle(BASELINE);
  const read = async (name) =>
    JSON.parse(
      await fs.readFile(`tests/migration/runtime/${name}-vectors.json`, "utf8"),
    );
  const staticCases = await read("static");
  assert.equal(staticCases.baseline, BASELINE);
  assert.equal(
    staticCases.exporter_sha256,
    digest(await fs.readFile("tests/migration/runtime/export-static.mjs")),
  );
  assert.equal(staticCases.cases.length, 217);
  const source = digest(
    await fs.readFile("tests/migration/runtime/logging-oracle.rs"),
  );
  const build = JSON.parse(
    await fs.readFile(
      "output/migration/execution/t11-logging-oracle-build.json",
      "utf8",
    ),
  );
  assert.equal(build.source_sha256, source);
  assert.equal(
    build.binary_sha256,
    digest(await fs.readFile("output/migration/execution/logging-oracle.exe")),
  );
  for (const dependency of build.dependencies)
    assert.equal(
      dependency.sha256,
      digest(await fs.readFile(dependency.path)),
      `Changed observer dependency: ${dependency.name}`,
    );
  const counts = { static: 217 };
  for (const name of ["log-regex", "log-filter"]) {
    const corpus = await read(name);
    assert.equal(corpus.provenance.source_sha256, source);
    assert.equal(corpus.provenance.binary_sha256, build.binary_sha256);
    assert.deepEqual(corpus.provenance.dependencies, build.dependencies);
    assert.equal(
      corpus.provenance.exporter_sha256,
      digest(await fs.readFile(`tests/migration/runtime/export-${name}.mjs`)),
    );
    counts[name] = corpus.cases.length;
  }
  const compact = await read("log-compact");
  assert.equal(compact.baseline, BASELINE);
  assert.equal(compact.observer_source_sha256, source);
  assert.equal(compact.observer_binary_sha256, build.binary_sha256);
  assert.equal(
    compact.exporter_sha256,
    digest(await fs.readFile("tests/migration/runtime/export-log-compact.mjs")),
  );
  counts.compact = compact.output.trimEnd().split("\n").length;
  assert.deepEqual(counts, {
    static: 217,
    "log-regex": 61,
    "log-filter": 165,
    compact: 3,
  });
  return {
    counts,
    oracleManifestSha256: digest(
      await fs.readFile(path.join(oracle.directory, "manifest.json")),
    ),
  };
}

/** Require actual native regressions rather than accepting a successful empty test selection. */
async function requireEvidence(result, platform) {
  const events = (await fs.readFile(result.log, "utf8"))
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  for (const name of [
    "TestExecutableOwnsPublicCommandsAndLogging",
    "TestCompactFormatMatchesOriginalObserver",
    "TestHttpPanicsCloseConnectionWithoutPayloadOrStack",
    "TestCapturedWorkerRetainsAncestryWithoutEnteringAncestorFilter",
    "TestCloseWaitsForCancelledCallerWorkerBeforeClosingRepository",
    "TestRunningServiceStopsAndDeletesOnlyOwnedApiHeartbeat",
    "TestFixtureSeedsRealStorageAndRefreshesOriginalOverHttp",
    "TestInternalChildLeavesIdleSidecarsForParent",
  ]) {
    assert(
      events.some((event) => event.Action === "pass" && event.Test === name),
      `Missing runtime regression: ${name}`,
    );
  }
  const executablePackage = "github.com/QianFuv/LitRadar/cmd/litradar";
  for (const name of [
    "same_binary_index_notify",
    ...(platform === "linux" ? ["real_sigterm", "blocked_stdin_signal"] : []),
  ]) {
    assert(
      events.some(
        (event) =>
          event.Package === executablePackage &&
          event.Action === "pass" &&
          event.Test === `TestExecutableOwnsPublicCommandsAndLogging/${name}`,
      ),
      `Missing actual process regression: ${name}`,
    );
  }
  const identities = events.filter(
    (event) =>
      event.Package === executablePackage &&
      event.Test === "TestExecutableOwnsPublicCommandsAndLogging" &&
      event.Action === "output" &&
      event.Output?.includes("NATIVE_SIMPLE "),
  );
  assert.equal(
    identities.length,
    1,
    "Actual native tokenizer identity is missing or ambiguous",
  );
  const native = identities[0];
  result.nativeLibrary = JSON.parse(
    native.Output.slice(native.Output.indexOf("NATIVE_SIMPLE ") + 14).trim(),
  );
  assert(result.nativeLibrary.path.length > 0);
  assert.match(result.nativeLibrary.sha256, /^[a-f0-9]{64}$/);
}

/** Execute bounded regular/race checks on both supported native systems and preserve input identities. */
export async function runRuntime() {
  const sourceIdentity = await identities(INPUTS),
    destination = "output/migration/execution/runtime-result.json";
  const report = {
    phase: "runtime",
    result: "In Progress",
    baseline: BASELINE,
    tags: "sqlite_fts5,sqlite_dbstat",
    sourceIdentity,
    commands: [],
  };
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Persist a managed check before execution and keep its complete result. */
  const record = async (id, executable, args) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args, 480000);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  try {
    await record("runtime-original-source", "git", [
      "diff",
      "--exit-code",
      BASELINE,
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
      ":(exclude)crates/litradar/examples/migration_fixture.rs",
    ]);
    report.dependencies = [
      await verifyDependency("go-sdk"),
      await verifyDependency("go-sqlite3"),
    ];
    report.inputs = await validateCorpora();
    await save();
    await record("runtime-format", "node", [
      "tests/migration/run.mjs",
      "--phase",
      "go-format",
    ]);
    for (const platform of ["windows", "linux"]) {
      /** Execute each OS's actual compiler with the accepted environment. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-runtime-${id}`, "go", args)
          : record(`linux-runtime-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "450s",
              "env",
              "GOWORK=off",
              "GOTOOLCHAIN=local",
              "GOENV=off",
              "GOFLAGS=",
              "CGO_ENABLED=1",
              "GOMODCACHE=/mnt/d/BuildCache/Go/modules",
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
      ]);
      await invoke("build-list", ["list", "-m", "-json", "all"]);
      let nativeLibrary;
      for (const isRace of [false, true]) {
        const result = await invoke(isRace ? "race" : "regular", [
          "test",
          "-json",
          "-count=1",
          "-mod=readonly",
          "-timeout=300s",
          ...(isRace ? ["-race"] : []),
          "-tags",
          "sqlite_fts5,sqlite_dbstat",
          ...PACKAGES,
        ]);
        await requireEvidence(result, platform);
        if (isRace)
          assert.deepEqual(
            result.nativeLibrary,
            nativeLibrary,
            `${platform} native library changed between regular and race checks`,
          );
        else nativeLibrary = result.nativeLibrary;
      }
    }
    await record("runtime-module-verify", "go", ["mod", "verify"]);
    await record("runtime-vet", "go", [
      "vet",
      "-tags",
      "sqlite_fts5,sqlite_dbstat",
      ...PACKAGES,
    ]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source inputs changed during V11",
    );
    report.result = "Passed";
    report.limitations = [
      "Race checks instrument Go test processes and exercised in-process services; public CLI child binaries are built normally in both runs.",
      "Frontend G2 is recorded separately by the complete public tests/test.mjs all run and generated API check.",
      "Production container packaging and removal of the original Rust source remain T12 and T14.",
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
