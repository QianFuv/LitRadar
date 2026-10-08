/**
 * Run the same uncached Go release checks on Linux and Windows.
 * Root suites allow 20 minutes per package for durable large-collection tests;
 * their 25-minute command budget also covers compilation and package scheduling.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { createHash, randomUUID } from "node:crypto";
import {
  buildSimple,
  simpleBuildEnvironment,
} from "./build-simple-tokenizer.mjs";

await buildSimple(process.cwd(), true);
const directory = path.resolve("test-results/go");
fs.mkdirSync(directory, { recursive: true });
const environment = {
  ...simpleBuildEnvironment(),
  CGO_ENABLED: "1",
  GOWORK: "off",
  GOENV: "off",
  GOFLAGS: "",
  GOTOOLCHAIN: "go1.27.2",
};
const tags = "sqlite_fts5,sqlite_dbstat";
const checks = [
  ["environment", "go", ["env", "-json"]],
  ["format", "node", ["scripts/check-go-format.mjs"]],
  ["modules", "go", ["mod", "verify"]],
  ["sdk", "node", ["scripts/verify-go-dependency.mjs", "go-sdk"]],
  ["sqlite", "node", ["scripts/verify-go-dependency.mjs", "go-sqlite3"]],
  ["vet", "go", ["vet", "-mod=readonly", "-tags", tags, "./..."]],
  [
    "regular",
    "go",
    [
      "test",
      "-count=1",
      "-timeout=20m",
      "-json",
      "-mod=readonly",
      "-tags",
      tags,
      "./...",
    ],
  ],
  [
    "race",
    "go",
    [
      "test",
      "-count=1",
      "-race",
      "-timeout=20m",
      "-json",
      "-mod=readonly",
      "-tags",
      tags,
      "./...",
    ],
  ],
];
/** Identify static archive, compiler metadata and SQLite headers for every release check. */
function nativeInputs() {
  const candidates = [
    "target/simple-tokenizer/libsimple.a",
    "target/simple-tokenizer/inputs.json",
    "third_party/go-sqlite3/sqlite3-binding.h",
    "third_party/go-sqlite3/sqlite3ext.h",
    "third_party/simple-static/CMakeLists.txt",
  ];
  return candidates.map((filename) => ({
    filename,
    sha256: createHash("sha256")
      .update(fs.readFileSync(filename))
      .digest("hex"),
  }));
}
const originalNativeInputs = nativeInputs();
assert.equal(
  originalNativeInputs.length,
  5,
  "Static tokenizer inputs are required",
);
fs.writeFileSync(
  path.join(directory, "native-inputs.json"),
  JSON.stringify(originalNativeInputs, null, 2) + "\n",
);
const results = [];
for (const mode of ["regular", "race"]) {
  const flags = [
    "test",
    "-count=1",
    "-json",
    "-mod=readonly",
    "-tags",
    tags,
    ...(mode === "race" ? ["-race"] : []),
  ];
  for (const [name, module, packageName] of [
    ["sdk", "go-sdk", "github.com/modelcontextprotocol/go-sdk/mcp"],
    ["sqlite", "go-sqlite3", "github.com/mattn/go-sqlite3"],
  ]) {
    checks.push([
      `${name}-own-${mode}`,
      "go",
      [
        "-C",
        `third_party/${module}`,
        ...flags,
        name === "sdk" ? "./mcp" : "./...",
      ],
    ]);
    checks.push([`${name}-root-${mode}`, "go", [...flags, packageName]]);
  }
}
for (const [id, executable, argumentsList] of checks) {
  const log = path.join(directory, `${id}.log`);
  const temporaryLog = path.join(
    os.tmpdir(),
    `litradar-go-${id}-${randomUUID()}.log`,
  );
  const descriptor = fs.openSync(temporaryLog, "wx", 0o600);
  const started = new Date().toISOString();
  const result = spawnSync(executable, argumentsList, {
    env: environment,
    windowsHide: true,
    timeout: id === "regular" || id === "race" ? 1500000 : 900000,
    stdio: ["ignore", descriptor, descriptor],
  });
  fs.closeSync(descriptor);
  fs.copyFileSync(temporaryLog, log);
  fs.unlinkSync(temporaryLog);
  results.push({
    id,
    executable,
    arguments: argumentsList,
    started,
    finished: new Date().toISOString(),
    status: result.status,
    error: result.error?.message,
    log,
  });
  fs.writeFileSync(
    path.join(directory, "results.json"),
    JSON.stringify(results, null, 2) + "\n",
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${id} failed; inspect ${log}`);
  if (id === "environment") {
    const actual = JSON.parse(fs.readFileSync(log, "utf8"));
    for (const [name, expected] of Object.entries({
      GOVERSION: "go1.27.2",
      CGO_ENABLED: "1",
      GOWORK: "off",
      GOFLAGS: "",
    }))
      assert.equal(actual[name], expected, `Unexpected ${name}`);
    const compiler = spawnSync(actual.CC, ["--version"], {
      encoding: "utf8",
      env: environment,
      timeout: 30000,
      windowsHide: true,
    });
    assert.ifError(compiler.error);
    assert.equal(compiler.status, 0, "C compiler unavailable");
    fs.writeFileSync(
      path.join(directory, "compiler.log"),
      compiler.stdout + compiler.stderr,
    );
  }
  assert.deepEqual(
    nativeInputs(),
    originalNativeInputs,
    "Native tokenizer changed during verification",
  );
  console.log(`${id}: Passed`);
}
