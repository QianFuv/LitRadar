/** Execute explicit migration proof phases; missing phase implementations fail closed. */
import assert from "node:assert/strict";
import {
  verifyFrozenEvidence,
  verifyHistoricalInput,
} from "./frozen-evidence.mjs";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest, loadOracle } from "./oracle.mjs";

/**
 * Parse the small migration runner interface without accepting duplicate options.
 * @param {string[]} args - CLI arguments.
 * @returns {Map<string,string>} Validated options.
 */
function options(args) {
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    assert(
      ["--phase", "--baseline", "--candidate", "--reuse-driver-proof"].includes(
        key,
      ),
      `Unknown option: ${key}`,
    );
    assert(
      !values.has(key) && args[index + 1] && !args[index + 1].startsWith("--"),
      `Invalid option: ${key}`,
    );
    values.set(key, args[index + 1]);
  }
  assert(values.has("--phase"), "--phase is required");
  return values;
}

const requested = options(process.argv.slice(2));
const phase = requested.get("--phase");
if (phase === "retirement") {
  assert.equal(
    requested.size,
    1,
    "Retirement executes its own clean-context checks",
  );
  const { runRetirement } = await import("./retirement.mjs");
  const result = await runRetirement();
  console.log(
    JSON.stringify({
      status: result.status,
      root: result.root,
      images: result.images,
    }),
  );
  process.exit(0);
}
if (["cutover", "rollback"].includes(phase)) {
  assert.equal(
    requested.size,
    1,
    "Cutover executes its own final-image checks",
  );
  const { runCutover } = await import("./cutover/run.mjs");
  const result = await runCutover(phase);
  console.log(
    JSON.stringify({
      status: result.status,
      image: result.image,
      runId: result.runId,
    }),
  );
  process.exit(0);
}
if (phase === "go-format") {
  assert.equal(requested.size, 1, "Formatting accepts only --phase");
  const files = [];
  /** Collect authored Go sources without vendored dependency rewrites. */
  async function collect(directory) {
    for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) await collect(filename);
      else if (entry.isFile() && entry.name.endsWith(".go"))
        files.push(filename);
    }
  }
  for (const directory of ["cmd", "internal"]) await collect(directory);
  const result = spawnSync("gofmt", ["-l", ...files.sort()], {
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    result.stdout.trim(),
    "",
    `Unformatted Go source:\n${result.stdout}`,
  );
  console.log(`Go formatting passed: ${files.length} files`);
  process.exit(0);
}
assert(
  !requested.has("--reuse-driver-proof") || phase === "auth",
  "Driver proof reuse is limited to auth",
);
assert.equal(requested.get("--baseline") ?? BASELINE, BASELINE);
if (phase === "cfp" || phase === "cfp-live") {
  assert(!requested.has("--candidate"), "CFP must execute its own checks");
  if (phase === "cfp") {
    const { runCfp } = await import("./cfp/run.mjs");
    console.log(await runCfp());
  } else {
    const { runCfpLive } = await import("./cfp/live.mjs");
    console.log(await runCfpLive());
  }
  process.exit(0);
}
if (["sdk-integrity", "sqlite-driver-integrity"].includes(phase)) {
  assert(
    !requested.has("--candidate"),
    "Integrity phase cannot accept candidate observations",
  );
  const { verifyDependency } = await import("./dependency.mjs");
  console.log(
    await verifyDependency(phase === "sdk-integrity" ? "go-sdk" : "go-sqlite3"),
  );
  process.exit(0);
}
if (phase === "primitives") {
  assert(
    !requested.has("--candidate"),
    "Primitives must execute their own checks",
  );
  const { runPrimitives } = await import("./primitives/run.mjs");
  console.log(await runPrimitives());
  process.exit(0);
}
if (phase === "auth") {
  assert(!requested.has("--candidate"), "Auth must execute its own checks");
  const { runAuth } = await import("./auth/run.mjs");
  console.log(await runAuth(requested.get("--reuse-driver-proof")));
  process.exit(0);
}
if (phase === "storage") {
  assert(!requested.has("--candidate"), "Storage must execute its own checks");
  const { runStorage } = await import("./storage/run.mjs");
  console.log(await runStorage());
  process.exit(0);
}
if (phase === "sources") {
  assert(
    !requested.has("--candidate"),
    "Sources must execute their own checks",
  );
  const { runSources } = await import("./sources/run.mjs");
  console.log(await runSources());
  process.exit(0);
}
if (phase === "index") {
  assert(!requested.has("--candidate"), "Index must execute its own checks");
  const { runIndex } = await import("./index/run.mjs");
  console.log(await runIndex());
  process.exit(0);
}
if (phase === "delivery") {
  assert(!requested.has("--candidate"), "Delivery must execute its own checks");
  const { runDelivery } = await import("./delivery/run.mjs");
  console.log(await runDelivery());
  process.exit(0);
}
if (phase === "scheduler") {
  assert(
    !requested.has("--candidate"),
    "Scheduler must execute its own checks",
  );
  const { runScheduler } = await import("./scheduler/run.mjs");
  console.log(await runScheduler());
  process.exit(0);
}
if (phase === "api") {
  assert(!requested.has("--candidate"), "API must execute its own checks");
  const { runApi } = await import("./api/run.mjs");
  console.log(await runApi());
  process.exit(0);
}
if (phase === "profile") {
  assert.equal(requested.size, 1, "Profile must execute its own image checks");
  const { profileImage } = await import("../profiling/go-image.mjs");
  try {
    console.log(await profileImage());
    process.exit(0);
  } catch (error) {
    console.error(error);
    process.exit(error.exitCode ?? 1);
  }
}
if (phase === "security") {
  assert.equal(requested.size, 1, "Security must execute its own checks");
  const { runSecurity } = await import("./security.mjs");
  console.log(await runSecurity());
  process.exit(0);
}
if (phase === "runtime") {
  assert(!requested.has("--candidate"), "Runtime must execute its own checks");
  const { runRuntime } = await import("./runtime/run.mjs");
  console.log(await runRuntime());
  process.exit(0);
}
assert.equal(
  phase,
  "baseline",
  `Phase ${phase} is not implemented; no parity result is available`,
);
assert(
  !requested.has("--candidate"),
  "Baseline phase cannot accept candidate observations",
);
const oracle = await loadOracle(BASELINE);
const fixtureRoot = path.join(WORKSPACE_ROOT, "tests/data/migration");
const inventory = JSON.parse(
  await fs.readFile(path.join(fixtureRoot, "inventory.json"), "utf8"),
);
const frontend = JSON.parse(
  await fs.readFile(path.join(fixtureRoot, "frontend-tests.json"), "utf8"),
);
const surfaces = JSON.parse(
  await fs.readFile(path.join(fixtureRoot, "surfaces.json"), "utf8"),
);
const portable = JSON.parse(
  await fs.readFile(path.join(fixtureRoot, "portable-fixtures.json"), "utf8"),
);
const evidence = JSON.parse(
  await fs.readFile(path.join(fixtureRoot, "baseline-evidence.json"), "utf8"),
);
for (const document of [inventory, frontend, surfaces, portable, evidence])
  assert.equal(document.baseline, BASELINE);
assert.equal(inventory.operations.length, 86);
assert.equal(Object.keys(inventory.schemas).length, 121);
assert.equal(inventory.runtimeSettings.length, 20);
assert.equal(inventory.rustTests.length, 1181);
assert.equal(frontend.layers.flatMap((layer) => layer.tests).length, 294);
assert.equal(surfaces.cli.length, 25);
assert.deepEqual(
  surfaces.mcp.tools.map((tool) => tool.name).sort(),
  inventory.mcp.map((tool) => tool.name).sort(),
);
for (const entry of [...inventory.rustTests, ...inventory.sourceOnlyTests]) {
  assert(
    entry.groups.length &&
      entry.owners.length &&
      entry.leaves.length &&
      entry.firstProof.length &&
      entry.finalIntegration &&
      entry.disposition &&
      entry.reason,
    `Unclassified test: ${entry.id ?? entry.name}`,
  );
}
for (const artifact of portable.files) {
  const original = oracle.manifest.files.find(
    (entry) => entry.path === artifact.oraclePath,
  );
  assert(
    original && original.sha256 === artifact.sha256,
    "Portable fixture lost independent provenance",
  );
  assert.equal(
    digest(await fs.readFile(path.join(fixtureRoot, artifact.path))),
    artifact.sha256,
    `Changed portable fixture: ${artifact.path}`,
  );
}
assert.equal(
  surfaces.oracleManifestSha256,
  digest(await fs.readFile(path.join(oracle.directory, "manifest.json"))),
);
assert.equal(evidence.checks.length, 12);
for (const check of evidence.checks) {
  const bytes = await fs.readFile(path.join(WORKSPACE_ROOT, check.log));
  assert.equal(
    digest(bytes),
    check.sha256,
    `Changed or missing evidence: ${check.id}`,
  );
  assert(
    check.exitCode === 0 || check.id === "baseline-all",
    `Failed required check: ${check.id}`,
  );
}
assert.equal(evidence.exception.id, "A3");
assert.equal(evidence.exception.files.length, 14);
await verifyFrozenEvidence();
for (const file of evidence.exception.files) await verifyHistoricalInput(file);
const controls = spawnSync(
  process.execPath,
  [
    "--test",
    "tests/migration/compare.test.mjs",
    "tests/migration/oracle.test.mjs",
    "tests/migration/exporter.test.mjs",
  ],
  {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    timeout: 60_000,
    maxBuffer: 4 * 1024 * 1024,
    shell: false,
  },
);
if (controls.error) throw controls.error;
process.stdout.write(controls.stdout);
assert.equal(controls.status, 0, controls.stderr);
const report = {
  phase,
  result: "Passed with A3 baseline-formatting exception",
  baseline: BASELINE,
  platform: process.platform,
  architecture: process.arch,
  oracleManifestSha256: surfaces.oracleManifestSha256,
  candidateParity: "Not Run",
  counts: {
    httpOperations: 86,
    schemas: 121,
    mcpTools: 13,
    runtimeSettings: 20,
    rustDiscovered: 1181,
    rustPassed: 1168,
    rustIgnored: 13,
    sourceOnly: inventory.sourceOnlyTests.length,
    frontendPassed: 294,
  },
  limitations: evidence.limitations,
  sourceIdentity: await Promise.all(
    [
      "run.mjs",
      "oracle.mjs",
      "compare.mjs",
      "compare.test.mjs",
      "oracle.test.mjs",
      "exporter.test.mjs",
    ].map(async (filename) => ({
      path: `tests/migration/${filename}`,
      sha256: digest(
        await fs.readFile(
          path.join(WORKSPACE_ROOT, "tests/migration", filename),
        ),
      ),
    })),
  ),
};
const destination = path.join(
  WORKSPACE_ROOT,
  "output/migration/execution/baseline-result.json",
);
await fs.writeFile(destination, `${JSON.stringify(report, null, 2)}\n`);
process.stdout.write(
  `${JSON.stringify(report.counts)}\nBaseline evidence validated; candidate parity remains Not Run.\n`,
);
