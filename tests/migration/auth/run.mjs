/** Execute fresh identity, persistence and bounded SQLite-patch proof on Windows and Linux. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/auth/...",
  "./internal/domain/auth/...",
  "./internal/storage/auth/...",
  "./internal/storage/secrets/...",
  "./internal/storage/settings/...",
  "./internal/storage/migrations/auth/...",
  "./internal/platform/sqlite/...",
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
const LINUX_GO = "/home/qianfuv/.cache/litradar-migration/go1.27.1/go/bin/go";
const LINUX_ENVIRONMENT = [
  "GOWORK=off",
  "GOTOOLCHAIN=local",
  "GOENV=off",
  "GOFLAGS=",
  "GOMODCACHE=/mnt/d/BuildCache/Go/modules",
  "GOCACHE=/home/qianfuv/.cache/litradar-migration/go-cache",
];

/** Collect exact authored and dependency bytes, including relevant untracked files. */
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

/** Validate frozen portable inputs and supplemental parser observations before executing Go. */
async function validateInputs() {
  const oracle = await loadOracle(BASELINE);
  const portable = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/data/migration/portable-fixtures.json"),
      "utf8",
    ),
  );
  assert.equal(portable.baseline, BASELINE);
  for (const fixture of portable.files) {
    const original = oracle.manifest.files.find(
      (item) => item.path === fixture.oraclePath,
    );
    assert(
      original && original.sha256 === fixture.sha256,
      "Portable fixture lost independent provenance",
    );
    assert.equal(
      digest(
        await fs.readFile(
          path.join(WORKSPACE_ROOT, "tests/data/migration", fixture.path),
        ),
      ),
      fixture.sha256,
    );
  }
  const settings = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/migration/auth/settings-vectors.json"),
      "utf8",
    ),
  );
  assert.equal(settings.baseline, BASELINE);
  assert(
    settings.cases.length >= 1524,
    "Settings differential corpus is incomplete",
  );
  for (const source of settings.sources.filter(
    (item) => !item.path.startsWith("target/"),
  ))
    assert.equal(
      digest(await fs.readFile(path.join(WORKSPACE_ROOT, source.path))),
      source.sha256,
      `Changed settings oracle source: ${source.path}`,
    );
  const urls = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/migration/auth/url-vectors.json"),
      "utf8",
    ),
  );
  assert.equal(urls.baseline, BASELINE);
  assert.equal(urls.cases.length, 3670);
  for (const source of urls.sources.filter(
    (item) => !item.path.startsWith("target/"),
  ))
    assert.equal(
      digest(await fs.readFile(path.join(WORKSPACE_ROOT, source.path))),
      source.sha256,
      `Changed URL oracle source: ${source.path}`,
    );
  return {
    oracleManifestSha256: digest(
      await fs.readFile(path.join(oracle.directory, "manifest.json")),
    ),
    settingsCases: settings.cases.length,
    urlCases: urls.cases.length,
  };
}

/** Run T03 tests and every A6 driver regression without sharing upstream SQLite temporary files. */
export async function runAuth(reuseDriverProof) {
  assert.equal(
    process.platform,
    "win32",
    "This proof route requires Windows with the approved WSL Linux toolchain",
  );
  const destination = path.join(
    WORKSPACE_ROOT,
    "output/migration/execution/auth-result.json",
  );
  const sourceIdentity = await identities(INPUTS);
  const inputs = await validateInputs();
  const dependency = await verifyDependency("go-sqlite3");
  const results = [];
  let prior;
  let priorSha256;
  if (reuseDriverProof) {
    const data = await fs.readFile(
      path.resolve(WORKSPACE_ROOT, reuseDriverProof),
    );
    priorSha256 = digest(data);
    prior = JSON.parse(data);
    assert.equal(prior.result, "Passed");
    assert.equal(prior.baseline, BASELINE);
    assert.deepEqual(prior.dependency, dependency);
    const closure = (entry) =>
      entry.path === "go.mod" ||
      entry.path === "go.sum" ||
      entry.path.startsWith("third_party/go-sqlite3") ||
      [
        "tests/migration/primitives/run.mjs",
        "tests/migration/dependency.mjs",
      ].includes(entry.path);
    assert.deepEqual(
      sourceIdentity.filter(closure),
      prior.sourceIdentity.filter(closure),
      "SQLite proof inputs changed",
    );
  }
  /** Record each completed proof immediately so interrupted execution remains recoverable. */
  async function record(id, executable, args) {
    let result;
    if (prior && /-auth-sqlite-(own|root)-(regular|race)$/.test(id)) {
      const original = prior.commands.find((item) => item.id === id);
      assert(
        original && !original.reuse,
        `Missing original driver proof: ${id}`,
      );
      assert.equal(original.exitCode, 0);
      assert(original.passedTests > 0);
      assert.equal(original.executable, executable);
      assert.deepEqual(original.args, args);
      assert.equal(
        digest(await fs.readFile(path.join(WORKSPACE_ROOT, original.log))),
        original.sha256,
      );
      const platform = id.split("-")[0];
      for (const suffix of [
        "environment",
        "build-list-root",
        "build-list-sqlite",
      ]) {
        const name = `${platform}-auth-${suffix}`;
        assert.equal(
          results.find((item) => item.id === name)?.sha256,
          prior.commands.find((item) => item.id === name)?.sha256,
          `Changed driver environment: ${name}`,
        );
      }
      result = {
        ...original,
        reuse: {
          result: "Reused",
          source: reuseDriverProof,
          sha256: priorSha256,
          reason:
            "Unchanged driver/source/helper/module closure and freshly identical OS environment/build lists",
        },
      };
      console.log(`${id}: Reused verified original proof`);
    } else result = await command(id, executable, args);
    results.push(result);
    await fs.writeFile(
      destination,
      JSON.stringify(
        {
          phase: "auth",
          result: "In Progress",
          baseline: BASELINE,
          inputs,
          dependency,
          sourceIdentity,
          commands: results,
        },
        null,
        2,
      ) + "\n",
    );
    return result;
  }
  await record("auth-integrity-controls", process.execPath, [
    "--test",
    "tests/migration/dependency.test.mjs",
  ]);
  await record("auth-secret-interoperability", process.execPath, [
    "tests/migration/auth/secret-interop.mjs",
  ]);
  for (const platform of ["windows", "linux"]) {
    /** Execute one Go command under the recorded OS environment with a ten-minute outer bound. */
    const invoke = (id, args) =>
      platform === "windows"
        ? record(`${platform}-auth-${id}`, "go", args)
        : record(`${platform}-auth-${id}`, "wsl", [
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
        "sqlite_fts5",
      ];
      await invoke(`application-${suffix}`, [...flags, ...PACKAGES]);
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
  await record("auth-module-verify", "go", ["mod", "verify"]);
  await record("auth-vet", "go", ["vet", "-tags", "sqlite_fts5", ...PACKAGES]);
  assert.deepEqual(
    await identities(INPUTS),
    sourceIdentity,
    "Auth proof inputs changed during verification",
  );
  const report = {
    phase: "auth",
    result: "Passed",
    baseline: BASELINE,
    inputs,
    dependency,
    sourceIdentity,
    commands: results,
    limitations: [
      "This proves T03 identity/storage behavior; HTTP, CLI, provider and runtime wiring remain assigned to later tasks.",
      "Parser differential cases are finite evidence, not an exhaustive proof over all input strings.",
    ],
  };
  await fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  return {
    result: report.result,
    commands: results.length,
    settingsCases: inputs.settingsCases,
  };
}
