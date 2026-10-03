/** Run the eight Go foundation experiments and both dependency build-list variants. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, BASELINE, digest, loadOracle } from "../oracle.mjs";
import { verifyDependency } from "../dependency.mjs";

const linuxGo = "/home/qianfuv/.cache/litradar-migration/go1.27.1/go/bin/go";
const linuxEnvironment = [
  "GOWORK=off",
  "GOTOOLCHAIN=local",
  "GOENV=off",
  "GOFLAGS=",
  "GOMODCACHE=/mnt/c/Users/57676/go/pkg/mod",
  "GOCACHE=/home/qianfuv/.cache/litradar-migration/go-cache",
];
const destination = path.join(WORKSPACE_ROOT, "output/migration/execution");
const results = [];

/**
 * Execute one bounded proof command and retain complete stdout/stderr as evidence.
 * @param {string} id - Unique proof identity.
 * @param {string} executable - Executable name.
 * @param {string[]} args - Literal argument list.
 * @param {number} timeout - Process deadline in milliseconds.
 * @returns {Promise<object>} Exit status and log identity.
 */
export async function command(id, executable, args, timeout = 600_000) {
  const started = new Date().toISOString();
  const chunks = [];
  const standardOutput = [];
  const child = spawn(executable, args, {
    cwd: WORKSPACE_ROOT,
    shell: false,
    windowsHide: true,
    env: {
      ...process.env,
      GOWORK: "off",
      GOTOOLCHAIN: "go1.27.1",
      GOENV: "off",
      GOFLAGS: "",
    },
  });
  let failure;
  child.stdout.on("data", (chunk) => {
    chunks.push(chunk);
    standardOutput.push(chunk);
  });
  child.stderr.on("data", (chunk) => chunks.push(chunk));
  let timer;
  const status = await new Promise((resolve, reject) => {
    timer = setTimeout(() => {
      failure = new Error(`Deadline exceeded: ${id}`);
      if (child.pid)
        spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
          windowsHide: true,
          timeout: 10_000,
          stdio: "ignore",
        });
      child.stdout.destroy();
      child.stderr.destroy();
      resolve(null);
    }, timeout);
    child.once("error", reject);
    child.once("close", resolve);
  }).finally(() => clearTimeout(timer));
  const log = `output/migration/execution/primitives-${id}.log`;
  const output = Buffer.concat(chunks);
  const stdout = Buffer.concat(standardOutput).toString("utf8");
  await fs.writeFile(path.join(WORKSPACE_ROOT, log), output);
  const result = {
    id,
    executable,
    args,
    started,
    finished: new Date().toISOString(),
    exitCode: status,
    log,
    sha256: digest(output),
  };
  if (status === 0 && /-(regular|race)$/.test(id)) {
    const events = stdout
      .split(/\r?\n/)
      .filter((line) => line.startsWith("{"))
      .map((line) => JSON.parse(line));
    result.passedTests = events.filter(
      (event) => event.Action === "pass" && event.Test,
    ).length;
    assert(result.passedTests > 0, `${id}: no tests executed`);
    assert(
      !events.some((event) => event.Action === "fail"),
      `${id}: failed test event`,
    );
  }
  if (status === 0 && id.endsWith("-environment")) {
    result.environment = JSON.parse(stdout);
    assert.equal(result.environment.GOVERSION, "go1.27.1");
    assert.equal(
      result.environment.GOOS,
      id.startsWith("windows-") ? "windows" : "linux",
    );
    assert.equal(result.environment.GOARCH, "amd64");
    assert.equal(result.environment.CGO_ENABLED, "1");
    assert.equal(result.environment.GOWORK, "off");
    assert.equal(result.environment.GOFLAGS, "");
    assert.equal(result.environment.CC, "gcc");
  }
  if (status === 0 && id === "runtime-base") {
    result.image = stdout.trim();
    assert.equal(
      result.image,
      "sha256:f25c66d32a1bdc4e4a8c9d81e492533938fdff3d0bf5319a5311734a7735bb23",
      "Unexpected helper runtime base",
    );
  }
  results.push(result);
  await fs.writeFile(
    path.join(destination, `primitives-${id}.json`),
    JSON.stringify(result, null, 2) + "\n",
  );
  console.log(`${id}: ${status === 0 && !failure ? "Passed" : "Failed"}`);
  if (failure) throw failure;
  assert.equal(status, 0, `${id} failed; see ${log}`);
  return result;
}

/**
 * Validate portable expectations against the separately pinned Rust oracle manifest.
 * @returns {Promise<string>} Verified oracle manifest digest.
 */
async function validateFrozenInputs() {
  const oracle = await loadOracle(BASELINE);
  const fixtures = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/data/migration/portable-fixtures.json"),
      "utf8",
    ),
  );
  assert.equal(fixtures.baseline, BASELINE);
  for (const fixture of fixtures.files) {
    const archived = oracle.manifest.files.find(
      (file) => file.path === fixture.oraclePath,
    );
    assert(
      archived && archived.sha256 === fixture.sha256,
      "Portable expected value lost independent provenance",
    );
    assert.equal(
      digest(
        await fs.readFile(
          path.join(WORKSPACE_ROOT, "tests/data/migration", fixture.path),
        ),
      ),
      fixture.sha256,
      `Changed expected fixture: ${fixture.path}`,
    );
  }
  return digest(
    await fs.readFile(path.join(oracle.directory, "manifest.json")),
  );
}

/**
 * Hash every relevant local implementation and fixture before running experiments.
 * @param {string[]} paths - Relative files or directory roots.
 * @returns {Promise<object[]>} Exact per-file identities.
 */
async function identities(paths) {
  const entries = [];
  for (const relative of paths.sort()) {
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

/**
 * Run all T02 platform/dependency proofs, then rebuild and exercise the hardened fixture image.
 * @returns {Promise<object>} Fresh, fail-closed primitive report.
 */
export async function runPrimitives() {
  assert.equal(
    process.platform,
    "win32",
    "This proof route requires the approved Windows plus WSL Linux host",
  );
  await fs.mkdir(destination, { recursive: true });
  const oracleManifestSha256 = await validateFrozenInputs();
  const paths = [
    "go.mod",
    "go.sum",
    "cmd",
    "internal",
    "third_party",
    "tests/data/migration",
    "tests/data/scenarios",
    "libs/simple",
    "tests/migration",
  ];
  const sourceIdentity = await identities(paths);
  const dependencyReports = [
    await verifyDependency("go-sdk"),
    await verifyDependency("go-sqlite3"),
  ];
  const jobs = [];
  let sqliteQueue = Promise.resolve();
  for (const platform of ["windows", "linux"]) {
    const invoke = (id, args) => {
      const run = () =>
        platform === "windows"
          ? command(`${platform}-${id}`, "go", args)
          : command(`${platform}-${id}`, "wsl", [
              "-d",
              "Ubuntu",
              "--exec",
              "timeout",
              "--signal=TERM",
              "--kill-after=5s",
              "540s",
              "env",
              ...linuxEnvironment,
              linuxGo,
              ...args,
            ]);
      if (!id.startsWith("sqlite-")) return run();
      const pending = sqliteQueue.then(run);
      sqliteQueue = pending.catch(() => {});
      return pending;
    };
    jobs.push(() =>
      invoke("environment", [
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
      ]),
    );
    for (const race of [false, true]) {
      const suffix = race ? "race" : "regular";
      const flags = [
        "test",
        "-json",
        "-count=1",
        "-mod=readonly",
        "-timeout=180s",
        ...(race ? ["-race"] : []),
      ];
      jobs.push(() =>
        invoke(`application-${suffix}`, [
          ...flags,
          "-tags",
          "sqlite_fts5",
          "./internal/platform/...",
          "./internal/domain/...",
        ]),
      );
      jobs.push(() =>
        invoke(`sdk-own-${suffix}`, [
          "-C",
          "third_party/go-sdk",
          ...flags,
          "./mcp",
        ]),
      );
      jobs.push(() =>
        invoke(`sdk-root-${suffix}`, [
          ...flags,
          "github.com/modelcontextprotocol/go-sdk/mcp",
        ]),
      );
      jobs.push(() =>
        invoke(`sqlite-own-${suffix}`, [
          "-C",
          "third_party/go-sqlite3",
          ...flags,
          "-tags",
          "sqlite_fts5",
          "./...",
        ]),
      );
      jobs.push(() =>
        invoke(`sqlite-root-${suffix}`, [
          ...flags,
          "-tags",
          "sqlite_fts5",
          "github.com/mattn/go-sqlite3",
        ]),
      );
    }
    for (const [id, prefix] of [
      ["root", []],
      ["sdk", ["-C", "third_party/go-sdk"]],
      ["sqlite", ["-C", "third_party/go-sqlite3"]],
    ])
      jobs.push(() =>
        invoke(`build-list-${id}`, [...prefix, "list", "-m", "-json", "all"]),
      );
  }
  let next = 0;
  const workers = await Promise.allSettled(
    Array.from({ length: 3 }, async () => {
      while (next < jobs.length) await jobs[next++]();
    }),
  );
  const errors = workers.filter((result) => result.status === "rejected");
  await fs.writeFile(
    path.join(destination, "primitives-command-results.json"),
    JSON.stringify(results, null, 2) + "\n",
  );
  if (errors.length)
    throw new AggregateError(
      errors.map((result) => result.reason),
      "Primitive commands failed",
    );
  await command("module-verify", "go", ["mod", "verify"]);
  await command("windows-compiler", "gcc", ["--version"]);
  await command("linux-compiler", "wsl", [
    "-d",
    "Ubuntu",
    "--exec",
    "gcc",
    "--version",
  ]);
  await command("runtime-base", "docker", [
    "image",
    "inspect",
    "litradar:migration-primitives-base",
    "--format",
    "{{.Id}}",
  ]);
  await command("fixture-build", "wsl", [
    "-d",
    "Ubuntu",
    "--exec",
    "timeout",
    "--signal=TERM",
    "--kill-after=5s",
    "540s",
    "env",
    ...linuxEnvironment,
    "CGO_ENABLED=0",
    linuxGo,
    "build",
    "-o",
    "output/migration/primitives/litradar-fixture",
    "./cmd/litradar-fixture",
  ]);
  await command("container-build", "docker", [
    "build",
    "-f",
    path.join(WORKSPACE_ROOT, "tests/migration/primitives/Dockerfile"),
    "-t",
    "litradar:migration-primitives",
    "output/migration/primitives",
  ]);
  await command(
    "container-run",
    process.execPath,
    ["tests/migration/primitives/container.mjs"],
    180_000,
  );
  const container = JSON.parse(
    await fs.readFile(
      path.join(destination, "t02-container-result.json"),
      "utf8",
    ),
  );
  assert(
    container.helpers.javascript &&
      container.helpers.originalHtml &&
      container.helpers.pdfCjk &&
      container.helpers.privateNetworkDenied,
  );
  assert(
    container.networkInternal &&
      container.fixedHost.fixedHostTls &&
      container.fixedHost.deliveryCount === 1 &&
      container.fixedHost.outboundDenied &&
      container.wrongCaRejected &&
      container.wrongSanRejected,
  );
  assert.deepEqual(
    await identities(paths),
    sourceIdentity,
    "Source inputs changed while checks were running",
  );
  const report = {
    phase: "primitives",
    baseline: BASELINE,
    oracleManifestSha256,
    result: "Passed",
    exceptions: [
      "A4: reject concurrent duplicate request ID with HTTP 400/-32600 without execution",
    ],
    experiments: [
      "E02-1",
      "E02-2",
      "E02-3",
      "E02-4",
      "E02-5",
      "E02-6",
      "E02-7",
      "E02-8",
    ],
    dependencyReports,
    sourceIdentity,
    commands: results,
    container,
    limitations: [
      "Private fixture listeners prove primitives; domain endpoints remain unimplemented.",
      "UNC string conversion is checked; no SMB server was used.",
      "Representative encoding and exhaustive GB18030 are proved; full provider encoding parity belongs to T09.",
    ],
  };
  await fs.writeFile(
    path.join(destination, "primitives-result.json"),
    JSON.stringify(report, null, 2) + "\n",
  );
  return {
    result: report.result,
    experiments: report.experiments,
    commands: results.length,
  };
}
