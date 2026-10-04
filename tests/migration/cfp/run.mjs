/** Verify original Rust cfp evidence and execute bounded native compatibility and recovery checks. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/cfp/...",
  "./internal/storage/cfp/...",
  "./internal/domain/cfp/...",
  "./internal/platform/...",
  "./internal/delivery/outbound/...",
];
const INPUTS = [
  "assets/cfp",
  "go.mod",
  "go.sum",
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
  storage: {
    count: 61,
    builder: "build-storage.mjs",
    exporter: "export-storage.mjs",
  },
  source: {
    count: 124,
    builder: "build-source.mjs",
    exporter: "export-source.mjs",
  },
  worker: {
    count: 43,
    builder: "build-worker.mjs",
    exporter: "export-worker.mjs",
  },
};

/** Hash all relevant tracked and untracked inputs, rejecting source-tree symlinks. */
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

/** Compare compilation inputs independently of nondeterministic executable linker identity. */
function compilationInputs(build) {
  const { binary_sha256, ...inputs } = build;
  assert.match(binary_sha256, /^[0-9a-f]{64}$/);
  return inputs;
}

/** Reject stale corpus exporters, source revisions, builders and linked locked dependencies. */
async function validateSources() {
  const original = await loadOracle(BASELINE),
    counts = {};
  for (const [name, specification] of Object.entries(CORPORA)) {
    const corpus = JSON.parse(
      await fs.readFile(`tests/migration/cfp/${name}-vectors.json`, "utf8"),
    );
    const build = JSON.parse(
      await fs.readFile(
        `output/migration/execution/cfp-oracle/${name}-build.json`,
        "utf8",
      ),
    );
    assert.equal(
      corpus.cases.length,
      specification.count,
      `Changed ${name} corpus inventory`,
    );
    assert.equal(
      corpus.exporter_sha256,
      digest(
        await fs.readFile(`tests/migration/cfp/${specification.exporter}`),
      ),
      `Stale ${name} exporter`,
    );
    assert.equal(
      build.builder_sha256,
      digest(await fs.readFile(`tests/migration/cfp/${specification.builder}`)),
      `Stale ${name} builder`,
    );
    assert.equal(
      build.binary_sha256,
      digest(await fs.readFile(build.binary)),
      `Changed ${name} observer binary`,
    );
    assert.deepEqual(
      compilationInputs(corpus.provenance),
      compilationInputs(build),
      `Stale ${name} compilation inputs`,
    );
    for (const input of [...build.inputs, ...build.dependencies])
      assert.equal(
        input.sha256,
        digest(await fs.readFile(input.path)),
        `Changed oracle input: ${input.path}`,
      );
    counts[name] = specification.count;
    if (name === "storage") {
      counts.storageTransitions = corpus.cases.reduce(
        (sum, entry) => sum + entry.steps.length,
        0,
      );
      assert.equal(counts.storageTransitions, 206);
    }
  }
  return {
    counts,
    oracleManifestSha256: digest(
      await fs.readFile(path.join(original.directory, "manifest.json")),
    ),
  };
}

/** Require actual behavior tests, process kills and platform-appropriate interoperation evidence. */
async function requireEvidence(result, platform) {
  const events = (await fs.readFile(result.log, "utf8"))
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  const required = [
    "TestOriginalSourceObservations",
    "TestOriginalWorkerObservations",
    "TestBundledSeedResumesHistoricalLineEndings",
    "TestOriginalStorageHistories",
    "TestConcurrentSeedAndGenerationFencing",
    "TestReadersSeeOnePublicationSnapshot",
    "TestHttpRedirectBoundariesAndWireBehavior",
    "TestHttpBodyBoundsAndNoPartialRetry",
    "TestHttpPublicDnsAndLifetime",
    "TestHttpConnectionCancellationAndDualStack",
    "TestHelperLifecycleAndWholeTreeOwnership",
    "TestObscuraProtocolAndOneAttempt",
    "TestPdfHelperFaithfulTextAndArguments",
    "TestCaptureResumeSkipsMalformedEntriesAndKeepsIdentity",
    "TestEvidenceFailurePreservesPublishedResult",
    "TestInvalidWorkerBudgetsDoNotImportSeed",
  ];
  if (platform === "windows")
    required.push("TestRustGoCfpDatabaseHandoffs", "TestRustGoCaptureHandoffs");
  for (const name of required)
    assert(
      events.some((event) => event.Action === "pass" && event.Test === name),
      `Missing ${platform} proof: ${name}`,
    );
  if (platform === "windows")
    for (const [prefix, count] of [
      ["TestRustGoCfpDatabaseHandoffs/", 12],
      ["TestRustGoCaptureHandoffs/", 6],
    ])
      assert.equal(
        events.filter(
          (event) =>
            event.Action === "pass" &&
            event.Test?.startsWith(prefix) &&
            event.Test.split("/").length === 3,
        ).length,
        count,
        "Both native handoff directions must execute",
      );
}

/** Execute V09 and retain incomplete state on interruption or a failed required check. */
export async function runCfp() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route uses Windows and the approved WSL Linux toolchain",
  );
  const sourceIdentity = await identities(INPUTS),
    destination = "output/migration/execution/cfp-result.json";
  const report = {
    phase: "cfp",
    result: "In Progress",
    baseline: BASELINE,
    tags: "sqlite_fts5",
    sourceIdentity,
    commands: [],
  };
  /** Persist operation identity before launch and consume only successful results. */
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Run one bounded command and retain its complete evidence log. */
  const record = async (id, executable, args) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args, 360000);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  const previousOracle = process.env.LITRADAR_CFP_ORACLE;
  const previousWorker = process.env.LITRADAR_CFP_WORKER_ORACLE;
  try {
    const inventory = await record("cfp-original-inventory", "git", [
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
    await record("cfp-original-source", "git", [
      "diff",
      "--exit-code",
      BASELINE,
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
      ":(exclude)crates/litradar/examples/migration_fixture.rs",
    ]);
    await record("cfp-original-fixture", "git", [
      "diff",
      "--exit-code",
      "9f305d9b71dc3ea6a739a6598aa762a4533203f0",
      "--",
      "crates/litradar/examples/migration_fixture.rs",
    ]);
    report.dependency = await verifyDependency("go-sqlite3");
    for (const [name, specification] of Object.entries(CORPORA))
      await record(`cfp-build-${name}`, process.execPath, [
        `tests/migration/cfp/${specification.builder}`,
      ]);
    report.inputs = await validateSources();
    await save();
    const owned = sourceIdentity
      .filter(
        (item) =>
          /^internal\/(cfp|storage\/cfp|domain\/cfp|platform|delivery\/outbound)\//.test(
            item.path,
          ) && item.path.endsWith(".go"),
      )
      .map((item) => item.path);
    const formatting = await record("cfp-formatting", "gofmt", [
      "-l",
      ...owned,
    ]);
    assert.equal(
      (await fs.readFile(formatting.log, "utf8")).trim(),
      "",
      "Unformatted Go source",
    );
    for (const platform of ["windows", "linux"]) {
      if (platform === "windows")
        process.env.LITRADAR_CFP_WORKER_ORACLE = path.resolve(
          "output/migration/execution/cfp-oracle/worker.exe",
        );
      else delete process.env.LITRADAR_CFP_WORKER_ORACLE;
      if (platform === "windows")
        process.env.LITRADAR_CFP_ORACLE = path.resolve(
          "output/migration/execution/cfp-oracle/storage.exe",
        );
      else delete process.env.LITRADAR_CFP_ORACLE;
      /** Run Linux checks with explicit native compiler/cache identity and a bounded lifetime. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-cfp-${id}`, "go", args)
          : record(`linux-cfp-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "330s",
              "env",
              "GOWORK=off",
              "GOTOOLCHAIN=local",
              "GOENV=off",
              "GOFLAGS=",
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
        await requireEvidence(result, platform);
      }
    }
    await record("cfp-module-verify", "go", ["mod", "verify"]);
    await record("cfp-vet", "go", ["vet", "-tags", "sqlite_fts5", ...PACKAGES]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source inputs changed during V09",
    );
    report.result = "Passed";
    report.limitations = [
      "T09 verifies native acquisition and storage. Service/CLI composition remains T10/T11; final image helper/container acceptance remains T12.",
      "Live Rust/Go database and capture handoffs execute on Windows; Linux executes original frozen corpora and native helper/race checks.",
      "Public live probes are separately reported by cfp-live; access failures never count as successful captures.",
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
  } finally {
    if (previousWorker === undefined)
      delete process.env.LITRADAR_CFP_WORKER_ORACLE;
    else process.env.LITRADAR_CFP_WORKER_ORACLE = previousWorker;
    if (previousOracle === undefined) delete process.env.LITRADAR_CFP_ORACLE;
    else process.env.LITRADAR_CFP_ORACLE = previousOracle;
  }
}
