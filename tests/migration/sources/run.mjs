/** Verify independent source provenance and execute the complete Windows/Linux provider proof. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { gunzipSync } from "node:zlib";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";
import { command } from "../primitives/run.mjs";

const PACKAGES = [
  "./internal/transport/...",
  "./internal/provider/...",
  "./internal/domain/sources/...",
  "./internal/sources/...",
];
const INPUTS = [
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
const CORPORA = [
  ["transport", "transport", "transport", 1975],
  ["number", "numbers", "transport", 1597],
  ["jfbym", "jfbym", "transport", 88],
  ["wire", "wire", "transport", 23],
  ["scholarly-decode", "scholarly-decode", "transport", 298],
  ["provider", "provider", "provider", 686],
  ["scholarly", "scholarly", "scholarly", 77],
  ["scheduler", "schedulers", "scheduler", 422],
  ["cnki", "cnki", "cnki", 655],
  ["zjlib", "zjlib", "zjlib", 503],
  ["workset", "workset", "workset", 361],
  ["workset-flow", "workset-flows", "workset", 19],
  ["access", "access", "access", 71],
  ["index", "index", "index", 1057],
  ["index-flow", "index-flows", "index", 30],
  ["cnki-index", "cnki-index", "index", 340],
  ["cnki-flow", "cnki-flows", "index", 28],
];

/** Hash all relevant tracked and untracked source bytes, rejecting filesystem links. */
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

/** Exclude compiler-produced executable identity while requiring identical source and dependency inputs. */
function compilationInputs(build) {
  const { binary_sha256, ...inputs } = build;
  assert.match(binary_sha256, /^[0-9a-f]{64}$/);
  return inputs;
}

/** Reject stale corpus, exporter, observer, source or dependency provenance. */
async function validateSources() {
  const original = await loadOracle(BASELINE),
    counts = {},
    oracles = {};
  for (const [name, exporter, oracle, count] of CORPORA) {
    let body = await fs.readFile(
      `tests/migration/sources/${name}-vectors.json${name === "scheduler" ? ".gz" : ""}`,
    );
    if (name === "scheduler") body = gunzipSync(body);
    const corpus = JSON.parse(body);
    assert.equal(
      corpus.observations.length,
      count,
      `Changed observation inventory: ${name}`,
    );
    assert.equal(
      corpus.exporter_sha256,
      digest(
        await fs.readFile(`tests/migration/sources/export-${exporter}.mjs`),
      ),
      `Stale exporter: ${name}`,
    );
    const current = JSON.parse(
      await fs.readFile(
        `output/migration/execution/t05-${oracle}-oracle-build.json`,
        "utf8",
      ),
    );
    assert.equal(
      current.builder_sha256,
      digest(await fs.readFile("tests/migration/sources/build-oracles.mjs")),
    );
    assert.deepEqual(
      compilationInputs(corpus.provenance),
      compilationInputs(current),
      `Stale oracle provenance: ${name}`,
    );
    assert.equal(
      current.source_sha256,
      digest(await fs.readFile(`tests/migration/sources/${oracle}-oracle.rs`)),
    );
    assert.equal(
      current.binary_sha256,
      digest(
        await fs.readFile(
          `output/migration/execution/sources-${oracle}-oracle.exe`,
        ),
      ),
    );
    for (const source of current.copiedSources)
      assert.equal(
        source.original_sha256,
        digest(await fs.readFile(source.path)),
        `Changed Rust input: ${source.path}`,
      );
    counts[name] = count;
    oracles[oracle] = current;
  }
  return {
    counts,
    oracles,
    oracleManifestSha256: digest(
      await fs.readFile(path.join(original.directory, "manifest.json")),
    ),
  };
}

/** Require actual bidirectional database handoff instead of a platform skip. */
async function requireWorksetHandoff(result) {
  const events = (await fs.readFile(result.log, "utf8"))
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line));
  assert(
    events.some(
      (event) =>
        event.Test === "TestCrossrefWorksetOriginalFileHandoff" &&
        event.Action === "pass",
    ),
    "Missing original Rust/Go workset handoff proof",
  );
}

/** Execute the approved T05 gate with immutable source identity and bounded test processes. */
export async function runSources() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route requires Windows and the approved WSL Linux toolchain",
  );
  const destination = "output/migration/execution/sources-result.json";
  const sourceIdentity = await identities(INPUTS);
  const report = {
    phase: "sources",
    result: "In Progress",
    baseline: BASELINE,
    tags: "sqlite_fts5",
    sourceIdentity,
    commands: [],
  };
  /** Persist state so an interrupted proof cannot retain an old success claim. */
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  await save();
  /** Record each process identity before starting and its complete evidence after completion. */
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
    const inventory = await record("sources-original-inventory", "git", [
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
    assert.deepEqual(actual, expected, "Original Rust input inventory changed");
    await record("sources-original-source", "git", [
      "diff",
      "--exit-code",
      BASELINE,
      "--",
      "crates",
      "Cargo.toml",
      "Cargo.lock",
      ":(exclude)crates/litradar/examples/migration_fixture.rs",
    ]);
    await record("sources-original-fixture", "git", [
      "diff",
      "--exit-code",
      "9f305d9b71dc3ea6a739a6598aa762a4533203f0",
      "--",
      "crates/litradar/examples/migration_fixture.rs",
    ]);
    report.dependency = await verifyDependency("go-sqlite3");
    await record("sources-build-oracles", process.execPath, [
      "tests/migration/sources/build-oracles.mjs",
    ]);
    report.inputs = await validateSources();
    await save();
    const owned = sourceIdentity
      .filter(
        (item) =>
          /^(internal\/(transport|provider|sources|domain\/sources)\/)/.test(
            item.path,
          ) && item.path.endsWith(".go"),
      )
      .map((item) => item.path);
    const formatting = await record("sources-formatting", "gofmt", [
      "-l",
      ...owned,
    ]);
    assert.equal(
      (await fs.readFile(formatting.log, "utf8")).trim(),
      "",
      "Unformatted source files",
    );
    for (const platform of ["windows", "linux"]) {
      /** Preserve the selected native toolchain and cap the complete Linux process tree. */
      const invoke = (id, args) =>
        platform === "windows"
          ? record(`windows-sources-${id}`, "go", args)
          : record(`linux-sources-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "540s",
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
          "-timeout=360s",
          ...(isRace ? ["-race"] : []),
          "-tags",
          "sqlite_fts5",
          ...PACKAGES,
        ]);
        if (platform === "windows") await requireWorksetHandoff(result);
      }
    }
    await record("sources-module-verify", "go", ["mod", "verify"]);
    await record("sources-vet", "go", [
      "vet",
      "-tags",
      "sqlite_fts5",
      ...PACKAGES,
    ]);
    assert.deepEqual(
      await identities(INPUTS),
      sourceIdentity,
      "Source proof inputs changed during execution",
    );
    report.result = "Passed";
    report.limitations = [
      "Provider, transport and disposable workset leaves only; API routing and runtime composition remain T10/T11.",
      "Live original Rust file handoff executes on Windows; Linux runs the same frozen fixtures and native/race checks.",
      "Network checks use real local HTTP/TLS/proxy servers; no production provider account was contacted.",
      "Finite differential observations do not prove every malformed HTML or URL input equivalent.",
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
