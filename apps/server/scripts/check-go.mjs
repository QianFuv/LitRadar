/**
 * Run complete uncached server-scoped Go verification, or one isolated regular/race CI lane.
 * Regular checks build without cgo; only race checks enable cgo, which Go's race detector
 * requires, so a C compiler is needed only when the selected lane includes race tests.
 * Root suites allow 20 minutes per package for durable large-collection tests;
 * their 25-minute command budget also covers compilation and package scheduling.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { randomUUID } from "node:crypto";
import { cgoSetting, goCheckPlan } from "./go-check-plan.mjs";

assert(process.argv.length <= 3, "Expected at most one Go check lane");
const checks = goCheckPlan(process.argv[2]);
const directory = path.resolve("test-results/go");
fs.mkdirSync(directory, { recursive: true });
const hasRaceChecks = checks.some(
  ([, , argumentsList]) => cgoSetting(argumentsList) === "1",
);
const environment = {
  ...process.env,
  CGO_ENABLED: "0",
  GOWORK: "off",
  GOENV: "off",
  GOFLAGS: "",
  GOTOOLCHAIN: "go1.27.3",
};
const results = [];
for (const [id, executable, argumentsList] of checks) {
  const log = path.join(directory, `${id}.log`);
  const temporaryLog = path.join(
    os.tmpdir(),
    `litradar-go-${id}-${randomUUID()}.log`,
  );
  const descriptor = fs.openSync(temporaryLog, "wx", 0o600);
  const started = new Date().toISOString();
  const result = spawnSync(executable, argumentsList, {
    env: { ...environment, CGO_ENABLED: cgoSetting(argumentsList) },
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
      GOVERSION: "go1.27.3",
      CGO_ENABLED: "0",
      GOWORK: "off",
      GOFLAGS: "",
    }))
      assert.equal(actual[name], expected, `Unexpected ${name}`);
    if (hasRaceChecks) {
      const compiler = spawnSync(actual.CC, ["--version"], {
        encoding: "utf8",
        env: environment,
        timeout: 30000,
        windowsHide: true,
      });
      assert.ifError(compiler.error);
      assert.equal(compiler.status, 0, "C compiler unavailable for race tests");
      fs.writeFileSync(
        path.join(directory, "compiler.log"),
        compiler.stdout + compiler.stderr,
      );
    }
  }
  console.log(`${id}: Passed`);
}
