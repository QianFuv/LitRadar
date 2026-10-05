/** Execute independent storage observations and current cross-platform native/race checks. */
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

const TAGS = "sqlite_fts5,sqlite_dbstat";
const PACKAGES = [
  "./internal/storage/...",
  "./internal/citation/...",
  "./internal/domain/storage/...",
];
const INPUTS = [
  "go.mod",
  "go.sum",
  "internal",
  "assets/meta",
  "third_party/go-sqlite3",
  "third_party/go-sqlite3-patches",
  "tests/data/migration",
  "tests/migration",
];
const LINUX_GO = "/home/qianfuv/.cache/litradar-migration/go1.27.1/go/bin/go";
const LINUX_ENVIRONMENT = [
  "GOWORK=off",
  "GOTOOLCHAIN=local",
  "GOENV=off",
  "GOFLAGS=",
  "GOMODCACHE=/mnt/d/BuildCache/Go/modules",
  "GOCACHE=/home/qianfuv/.cache/litradar-migration/go-cache",
];

/** Collect all relevant tracked and untracked bytes without following source links. */
async function identities(paths) {
  const entries = [];
  for (const relative of [...paths].sort()) {
    const filename = path.join(WORKSPACE_ROOT, relative);
    const metadata = await fs.lstat(filename);
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

/** Reject stale original-source or expected-fixture identities before testing a candidate. */
export async function validateStorageInputs() {
  await verifyFrozenEvidence();
  const oracle = await loadOracle(BASELINE);
  const counts = {};
  for (const name of [
    "citation",
    "index",
    "search",
    "metadata",
    "weekly",
    "weekly-query",
    "favorites",
    "backup",
  ]) {
    const document = JSON.parse(
      await fs.readFile(
        path.join(
          WORKSPACE_ROOT,
          `tests/migration/storage/${name}-vectors.json`,
        ),
        "utf8",
      ),
    );
    assert.equal(document.baseline, BASELINE);
    assert(document.cases.length > 0, `Empty corpus: ${name}`);
    counts[name] = document.cases.length;
    for (const source of document.sources) await verifyHistoricalInput(source);
    for (const source of document.visibility_only_sources ?? [])
      await verifyHistoricalInput({
        path: source.path,
        sha256: source.original_sha256,
      });
    if (name === "index")
      for (const fixture of document.cases)
        assert.equal(
          digest(
            await fs.readFile(
              path.join(
                WORKSPACE_ROOT,
                "tests/migration/storage/fixtures",
                fixture.file,
              ),
            ),
          ),
          fixture.sha256,
        );
    if (name === "metadata" || name === "favorites")
      assert.equal(
        digest(
          await fs.readFile(
            path.join(
              WORKSPACE_ROOT,
              "tests/migration/storage/fixtures",
              name === "metadata"
                ? "metadata.sqlite.fixture"
                : "favorites-auth.sqlite.fixture",
            ),
          ),
        ),
        document.fixture_sha256,
      );
    if (name === "search") assert.equal(document.scalar_blocks.length, 272);
  }
  return {
    cases: counts,
    oracleManifestSha256: digest(
      await fs.readFile(path.join(oracle.directory, "manifest.json")),
    ),
  };
}

/** Require live bidirectional operations, never a silently skipped interoperability check. */
async function requireInteroperability(result) {
  const events = (
    await fs.readFile(path.join(WORKSPACE_ROOT, result.log), "utf8")
  )
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  for (const name of [
    "TestGoBackupVerifiedAndRestoredByOriginalRust",
    "TestOriginalRustReadsGoUpgradeAndGoReadsRustWrite",
    "TestOriginalRustAndGoExchangeOptimizedDatabases",
  ])
    assert(
      events.some((event) => event.Test === name && event.Action === "pass"),
      `Required interoperability test did not pass: ${name}`,
    );
}

/** Run T04 proof with the same FTS/dbstat features required by the final application. */
export async function runStorage() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route requires Windows and the approved WSL Linux toolchain",
  );
  const destination = path.join(
    WORKSPACE_ROOT,
    "output/migration/execution/storage-result.json",
  );
  const sourceIdentity = await identities(INPUTS);
  const report = {
    phase: "storage",
    result: "In Progress",
    baseline: BASELINE,
    tags: TAGS,
    sourceIdentity,
    commands: [],
  };
  /** Persist progress after each completed command; a failure cannot leave an old success report. */
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Execute a bounded command and persist its exact log and argument identity. */
  const record = async (id, executable, args) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  const priorBackupOracle = process.env.LITRADAR_RUST_STORAGE_BACKUP_ORACLE;
  const priorDatabaseOracle = process.env.LITRADAR_RUST_STORAGE_DATABASE_ORACLE;
  try {
    report.inputs = await validateStorageInputs();
    report.dependency = await verifyDependency("go-sqlite3");
    report.oracles = [];
    for (const name of ["backup", "database"]) {
      const filename = `output/migration/execution/${name}-oracle.exe`;
      const build = JSON.parse(
        await fs.readFile(
          path.join(
            WORKSPACE_ROOT,
            `output/migration/execution/t04-${name}-oracle-build.json`,
          ),
          "utf8",
        ),
      );
      assert.equal(
        digest(await fs.readFile(path.join(WORKSPACE_ROOT, filename))),
        build.binary_sha256,
      );
      report.oracles.push({ name, ...build });
    }
    process.env.LITRADAR_RUST_STORAGE_BACKUP_ORACLE = path.join(
      WORKSPACE_ROOT,
      "output/migration/execution/backup-oracle.exe",
    );
    process.env.LITRADAR_RUST_STORAGE_DATABASE_ORACLE = path.join(
      WORKSPACE_ROOT,
      "output/migration/execution/database-oracle.exe",
    );
    await record("storage-integrity-controls", process.execPath, [
      "--test",
      "tests/migration/dependency.test.mjs",
    ]);
    for (const platform of ["windows", "linux"]) {
      /** Keep compiler, environment and build-list selection explicit for each platform. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`${platform}-storage-${id}`, "go", args)
          : record(`${platform}-storage-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "540s",
              "env",
              ...LINUX_ENVIRONMENT,
              LINUX_GO,
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
      for (const [role, prefix] of [
        ["root", []],
        ["sqlite", ["-C", "third_party/go-sqlite3"]],
      ])
        await invoke(`build-list-${role}`, [
          ...prefix,
          "list",
          "-m",
          "-json",
          "all",
        ]);
      for (const isRace of [false, true]) {
        const suffix = isRace ? "race" : "regular";
        const flags = [
          "test",
          "-json",
          "-count=1",
          "-mod=readonly",
          "-timeout=180s",
          ...(isRace ? ["-race"] : []),
          "-tags",
          TAGS,
        ];
        const application = await invoke(`application-${suffix}`, [
          ...flags,
          ...PACKAGES,
        ]);
        if (platform === "windows") await requireInteroperability(application);
        await invoke(`sqlite-own-${suffix}`, [
          "-C",
          "third_party/go-sqlite3",
          ...flags,
          "./...",
        ]);
        await invoke(`sqlite-root-${suffix}`, [
          ...flags,
          "github.com/mattn/go-sqlite3",
        ]);
      }
    }
    await record("storage-module-verify", "go", ["mod", "verify"]);
    await record("storage-vet", "go", ["vet", "-tags", TAGS, ...PACKAGES]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Storage proof inputs changed during execution",
    );
    report.result = "Passed";
    report.limitations = [
      "T04 storage leaves only; runtime/API/provider wiring remains assigned to later tasks.",
      "Live original Rust interoperability executes on Windows; Linux independently runs frozen original fixtures and race/native checks.",
      "Finite differential observations do not establish equivalence for every possible malformed input.",
    ];
    await save();
    return {
      result: report.result,
      commands: report.commands.length,
      cases: report.inputs.cases,
    };
  } catch (error) {
    report.result = "Failed";
    report.error = String(error);
    await save();
    throw error;
  } finally {
    if (priorBackupOracle === undefined)
      delete process.env.LITRADAR_RUST_STORAGE_BACKUP_ORACLE;
    else process.env.LITRADAR_RUST_STORAGE_BACKUP_ORACLE = priorBackupOracle;
    if (priorDatabaseOracle === undefined)
      delete process.env.LITRADAR_RUST_STORAGE_DATABASE_ORACLE;
    else
      process.env.LITRADAR_RUST_STORAGE_DATABASE_ORACLE = priorDatabaseOracle;
  }
}
