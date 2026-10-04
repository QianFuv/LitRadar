/** Verify frozen original HTTP/MCP contracts and current native API composition. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/api/...",
  "./internal/mcp/...",
  "./internal/openapi/...",
  "./internal/domain/api/...",
  "./internal/platform/...",
  "./internal/storage/auth/...",
];
const INPUTS = [
  "go.mod",
  "go.sum",
  "internal",
  "third_party/go-sdk",
  "third_party/go-sdk-patches",
  "third_party/go-sqlite3",
  "third_party/go-sqlite3-patches",
  "tests/data/migration",
  "tests/migration",
  "crates",
  "Cargo.toml",
  "Cargo.lock",
];
const COUNTS = {
  "http-favorite": 44,
  "http-json": 151,
  "http-query": 56,
  "mcp-float": 3,
  mcp: 77,
  router: 177,
};

/** Hash relevant sources, including untracked inputs, without following source symlinks. */
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

/** Verify the independent original manifest, exporter identities and frozen fixture bytes. */
async function validateCorpora() {
  const oracle = await loadOracle(BASELINE);
  for (const [name, count] of Object.entries(COUNTS)) {
    const corpus = JSON.parse(
      await fs.readFile(`tests/migration/api/${name}-vectors.json`, "utf8"),
    );
    assert.equal(corpus.baseline, BASELINE);
    assert.equal(corpus.cases.length, count);
    assert.equal(
      corpus.exporter_sha256,
      digest(
        await fs.readFile(
          `tests/migration/api/export-${name === "router" ? "router" : "mcp"}.mjs`,
        ),
      ),
      `Stale ${name} observations`,
    );
    for (const fixture of corpus.fixtures ?? [])
      assert.equal(
        fixture.sha256,
        digest(await fs.readFile(fixture.path)),
        `Changed ${fixture.path}`,
      );
  }
  const cfp = JSON.parse(
    await fs.readFile("tests/migration/api/cfp-vectors.json", "utf8"),
  );
  assert.equal(cfp.baseline, BASELINE);
  assert.equal(cfp.errors.length + 3, 12);
  assert.equal(
    cfp.exporter_sha256,
    digest(await fs.readFile("tests/migration/api/export-cfp.mjs")),
  );
  return {
    counts: { ...COUNTS, cfp: 12 },
    oracleManifestSha256: digest(
      await fs.readFile(path.join(oracle.directory, "manifest.json")),
    ),
  };
}

/** Require executed behavior proofs rather than accepting an empty package pass. */
async function requireEvidence(result) {
  const events = (await fs.readFile(result.log, "utf8"))
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  for (const name of [
    "TestCompleteRouterMatchesOriginalWire",
    "TestCompleteRouterMcpAuthenticationAndStreaming",
    "TestMiddlewareFlushesBeforeStreamCompletion",
    "TestRouterSecurityCorsCacheAndPrivateLogs",
    "TestDocumentRetainsFrozenContractAndRejectsMissingBindings",
    "TestAuthenticatedToolsUseFrozenContractsAndCurrentIdentity",
    "TestJsonExtractionMatchesOriginalRawRequests",
    "TestAuthRouteLifecycleAndNumericUserIdentity",
    "TestAuditFailureIsCountedOnceAndStillLogged",
    "TestLogoutUnconfirmedClearsCookieAndKeepsDedicatedError",
    "TestFavoriteRoutesMatchOriginalResponses",
    "TestFavoriteValidationPrecedesBusinessAdmission",
    "TestFavoriteMoveValidatesItemsBeforeSameFolder",
    "TestCfpMatchesOriginalResponsesAndReadsOriginalCursor",
    "TestArticleSharedDeadlineDiscardsLateResultsAndPreservesLoginPriority",
    "TestCnkiLateCompletionCannotResurrectClearedSession",
    "TestManualUnknownAcknowledgementRemainsOwnerOnlyAndAtomic",
    "TestTrackingNormalizedEndpointLengthRemainsClientError",
    "TestAdministratorTaskAuditFailureRollsBackCreate",
    "TestAdministratorRuntimeSettingsValidateRawCapabilitiesAndMaskSecrets",
    "TestCancelledRequestRetainsCapacityAndCloseWakesWaiters",
  ]) {
    assert(
      events.some((event) => event.Action === "pass" && event.Test === name),
      `Missing native proof: ${name}`,
    );
  }
  assert.equal(
    events.filter(
      (event) =>
        event.Action === "pass" &&
        event.Test?.startsWith("TestCompleteRouterMatchesOriginalWire/"),
    ).length,
    177,
  );
}

/** Execute bounded Windows/Linux regular, race, dependency, schema and middleware checks. */
export async function runApi() {
  const sourceIdentity = await identities(INPUTS),
    destination = "output/migration/execution/api-result.json";
  const report = {
    phase: "api",
    result: "In Progress",
    baseline: BASELINE,
    tags: "sqlite_fts5",
    sourceIdentity,
    commands: [],
  };
  /** Persist the recoverable operation identity before and after each check. */
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Run one check with a six-minute process bound and keep its complete evidence. */
  const record = async (id, executable, args) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args, 360000);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  try {
    await record("api-original-source", "git", [
      "diff",
      "--exit-code",
      BASELINE,
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
      ":(exclude)crates/litradar/examples/migration_fixture.rs",
    ]);
    await record("api-original-fixture", "git", [
      "diff",
      "--exit-code",
      "9f305d9b71dc3ea6a739a6598aa762a4533203f0",
      "--",
      "crates/litradar/examples/migration_fixture.rs",
    ]);
    report.dependencies = [
      await verifyDependency("go-sdk"),
      await verifyDependency("go-sqlite3"),
    ];
    report.inputs = await validateCorpora();
    await save();
    const owned = sourceIdentity
      .filter(
        (entry) =>
          /^internal\/(api|mcp|openapi|domain\/api|platform|storage\/auth)\//.test(
            entry.path,
          ) && entry.path.endsWith(".go"),
      )
      .map((entry) => entry.path);
    const formatting = await record("api-formatting", "gofmt", [
      "-l",
      ...owned,
    ]);
    assert.equal(
      (await fs.readFile(formatting.log, "utf8")).trim(),
      "",
      "Unformatted Go source",
    );
    for (const platform of ["windows", "linux"]) {
      /** Pin native compiler/cache identity independently on each supported OS. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-api-${id}`, "go", args)
          : record(`linux-api-${id}`, "wsl", [
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
        await requireEvidence(result);
      }
    }
    await record("api-module-verify", "go", ["mod", "verify"]);
    await record("api-vet", "go", ["vet", "-tags", "sqlite_fts5", ...PACKAGES]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source inputs changed during V10",
    );
    report.result = "Passed";
    report.limitations = [
      "T10 validates composed HTTP/MCP services and fixed schemas. Public CLI, production static-export/CSP loading, heartbeat and shutdown ownership remain T11.",
      "Frozen original Windows responses execute on both OSes; no upstream live CNKI availability claim is made by local protocol tests.",
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
