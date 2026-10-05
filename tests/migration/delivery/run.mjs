/** Verify original Rust delivery evidence and execute bounded native compatibility and recovery checks. */
import assert from "node:assert/strict";
import {
  verifyFrozenEvidence,
  verifyHistoricalInput,
} from "../frozen-evidence.mjs";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/recommend/...",
  "./internal/delivery/...",
  "./internal/storage/delivery/...",
  "./internal/domain/delivery/...",
];
const INPUTS = [
  "go.mod",
  "go.sum",
  "internal",
  "third_party/go-sqlite3",
  "third_party/go-sqlite3-patches",
  "tests/data/migration",
  "tests/migration",
];
const CORPORA = {
  recommend: {
    count: 260,
    builder: "build-oracles.mjs",
    exporter: "export-recommend.mjs",
  },
  legacy: {
    count: 52,
    builder: "build-legacy.mjs",
    exporter: "export-legacy.mjs",
  },
  client: {
    count: 81,
    builder: "build-clients.mjs",
    exporter: "export-clients.mjs",
  },
  durable: {
    count: 63,
    builder: "build-durable.mjs",
    exporter: "export-durable.mjs",
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
export async function validateSources() {
  await verifyFrozenEvidence();
  const original = await loadOracle(BASELINE),
    counts = {};
  for (const [name, specification] of Object.entries(CORPORA)) {
    const corpus = JSON.parse(
      await fs.readFile(
        `tests/migration/delivery/${name}-vectors.json`,
        "utf8",
      ),
    );
    const build = JSON.parse(
      await fs.readFile(
        `output/migration/execution/delivery-oracle/${name}-build.json`,
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
        await fs.readFile(`tests/migration/delivery/${specification.exporter}`),
      ),
      `Stale ${name} exporter`,
    );
    assert.equal(
      build.builder_sha256,
      digest(
        await fs.readFile(`tests/migration/delivery/${specification.builder}`),
      ),
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
      await verifyHistoricalInput(input);
    counts[name] = specification.count;
    if (name === "durable") {
      counts.durableTransitions = corpus.cases.reduce(
        (sum, entry) => sum + entry.steps.length,
        0,
      );
      assert.equal(counts.durableTransitions, 315);
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
    "TestOriginalRecommendationObservations",
    "TestOriginalAiClientObservations",
    "TestOriginalPushplusClientObservations",
    "TestOriginalLegacyImportHistories",
    "TestOriginalRustDurableHistories",
    "TestUnknownManualAcknowledgmentRequiresAtomicAudit",
    "TestWorkflowDryRunPersistsProgressWithoutFavoritesDedupeOrSend",
    "TestWorkflowPostSendCommitFailureCannotCauseReplay",
    "TestWorkflowRecoversSendingItemFromDurableInputsWithoutCallingProviders",
    "TestDeliveryProcessKillPreservesEffectsAndQuarantinesAmbiguity",
    "TestManualJobPersistsCancellationAfterExecutionContextEnds",
    "TestManualWeeklyUsesFixedMembershipAndStableAttempt",
    "TestManualWeeklySharesBudgetAndStopsAfterFailure",
    "TestWorkflowFailureLogsExcludeProviderAndSubscriberPayloads",
    "TestAiRechecksAllowlistBeforeEachAttempt",
    "TestAiBudgetIsSharedAcrossFormatsAndCompletions",
    "TestOneBudgetSurvivesConcurrentEndpointAndFormatAttempts",
    "TestTimedOutDnsStillHoldsOneOfEightLookupSlots",
  ];
  if (platform === "windows")
    required.push("TestRustGoDurableDatabaseHandoffs");
  for (const name of required)
    assert(
      events.some((event) => event.Action === "pass" && event.Test === name),
      `Missing ${platform} proof: ${name}`,
    );
  assert.equal(
    events.filter(
      (event) =>
        event.Action === "pass" &&
        event.Test?.startsWith(
          "TestDeliveryProcessKillPreservesEffectsAndQuarantinesAmbiguity/",
        ),
    ).length,
    5,
    "Every process crash boundary must execute",
  );
  if (platform === "windows")
    assert.equal(
      events.filter(
        (event) =>
          event.Action === "pass" &&
          event.Test?.startsWith("TestRustGoDurableDatabaseHandoffs/"),
      ).length,
      12,
      "Both handoff directions must execute",
    );
}

/** Execute V07 and retain incomplete state on interruption or a failed required check. */
export async function runDelivery() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route uses Windows and the approved WSL Linux toolchain",
  );
  const sourceIdentity = await identities(INPUTS),
    destination = "output/migration/execution/delivery-result.json";
  const report = {
    phase: "delivery",
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
  const previousOracle = process.env.LITRADAR_DELIVERY_ORACLE;
  try {
    report.dependency = await verifyDependency("go-sqlite3");
    report.inputs = await validateSources();
    await save();
    const owned = sourceIdentity
      .filter(
        (item) =>
          /^internal\/(recommend|delivery|storage\/delivery|domain\/delivery)\//.test(
            item.path,
          ) && item.path.endsWith(".go"),
      )
      .map((item) => item.path);
    const formatting = await record("delivery-formatting", "gofmt", [
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
        process.env.LITRADAR_DELIVERY_ORACLE = path.resolve(
          "output/migration/execution/delivery-oracle/durable.exe",
        );
      else delete process.env.LITRADAR_DELIVERY_ORACLE;
      /** Run Linux checks with explicit native compiler/cache identity and a bounded lifetime. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-delivery-${id}`, "go", args)
          : record(`linux-delivery-${id}`, "wsl", [
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
    await record("delivery-module-verify", "go", ["mod", "verify"]);
    await record("delivery-vet", "go", [
      "vet",
      "-tags",
      "sqlite_fts5",
      ...PACKAGES,
    ]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source inputs changed during V07",
    );
    report.result = "Passed";
    report.limitations = [
      "T07 implements delivery and manual jobs; public HTTP/CLI and dispatcher wiring remain T10/T11.",
      "Live Rust/Go file handoff executes on Windows; Linux executes frozen Rust state histories and native process-recovery/race checks.",
      "Process crash tests force-kill synthetic workers and explicitly expire retained leases before recovery.",
      "Windows-specific path behavior is tested on Windows; Linux uses its native path semantics.",
      "Malformed manifest diagnostics preserve failure classification and pre-admission behavior, not internal serde error wording.",
      "Finite independent observations do not prove every possible malformed input; all provider calls use synthetic local transports or fixtures.",
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
    if (previousOracle === undefined)
      delete process.env.LITRADAR_DELIVERY_ORACLE;
    else process.env.LITRADAR_DELIVERY_ORACLE = previousOracle;
  }
}
