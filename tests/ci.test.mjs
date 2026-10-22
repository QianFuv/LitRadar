/** Preserve the complete quality contract when scheduling independent CI lanes. */
import assert from "node:assert/strict";
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { goCheckPlan } from "../scripts/go-check-plan.mjs";

test("regular and race lanes retain every root and replacement-module check", () => {
  const expected = [
    "environment",
    "format",
    "modules",
    "sdk",
    "sqlite",
    "vet",
    "regular",
    "race",
    "sdk-own-regular",
    "sdk-root-regular",
    "sqlite-own-regular",
    "sqlite-root-regular",
    "sdk-own-race",
    "sdk-root-race",
    "sqlite-own-race",
    "sqlite-root-race",
  ].sort();
  assert.deepEqual(
    goCheckPlan()
      .map(([id]) => id)
      .sort(),
    expected,
  );
  const lanes = [goCheckPlan("regular"), goCheckPlan("race")];
  assert.deepEqual(
    [...new Set(lanes.flat().map(([id]) => id))].sort(),
    expected,
  );
  for (const mode of ["regular", "race"]) {
    const checks = goCheckPlan(mode);
    for (const [id, , args] of checks.filter(([, , args]) =>
      args.includes("test"),
    )) {
      assert(args.includes("-count=1"), id);
      assert(args.includes("-mod=readonly"), id);
      assert(args.includes("sqlite_fts5,sqlite_dbstat"), id);
      assert(!args.includes("-short"), id);
      assert.equal(args.includes("-race"), mode === "race", id);
      if (id === mode) assert(args.includes("-timeout=20m"), id);
      if (id !== "regular") assert(!args.includes("-vet=off"), id);
    }
  }
  const regular = goCheckPlan("regular");
  assert(
    regular.findIndex(([id]) => id === "vet") <
      regular.findIndex(([id]) => id === "regular"),
  );
  assert.equal(
    goCheckPlan("race").filter(([, , args]) => args.includes("test")).length,
    5,
  );
});

test("unknown lanes fail before native preparation", () => {
  const result = spawnSync(
    process.execPath,
    ["scripts/check-go.mjs", "short"],
    {
      encoding: "utf8",
      timeout: 10000,
      windowsHide: true,
    },
  );
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown Go check lane/);
  assert.equal(result.stdout, "");
});

test("release prerequisites include every lane and obsolete quality runs are isolated from releases", () => {
  const ci = fs.readFileSync(".github/workflows/ci.yaml", "utf8");
  const backend = fs.readFileSync(".github/workflows/backend.yaml", "utf8");
  assert.match(backend, /lane: \[regular, race\]/);
  assert.match(backend, /os: \[ubuntu-26\.04, windows-latest\]/);
  assert.match(backend, /fail-fast: false/);
  assert.match(backend, /needs: checks\s+if: always\(\)/);
  assert.match(backend, /test "\$CHECKS_RESULT" = success/);
  assert.match(ci, /needs: \[release-rules, backend, frontend\]/);
  for (const name of ["backend", "frontend"]) {
    assert.match(
      ci,
      new RegExp(
        `  ${name}:\\s+name: \\w+\\s+needs: release-rules\\s+if: \\$\\{\\{ !cancelled\\(\\) \\}\\}`,
      ),
      "Quality workflows must still execute when release-rule verification fails",
    );
    assert.match(
      ci,
      new RegExp(
        `group: ${name}-.*format\\('release-\\{0\\}', github.run_id\\).*format\\('quality-\\{0\\}-\\{1\\}'`,
      ),
    );
    const reusable = fs.readFileSync(`.github/workflows/${name}.yaml`, "utf8");
    assert(
      !/^concurrency:/m.test(reusable),
      "Only the caller owns cancellation",
    );
  }
  assert.match(
    backend,
    /key: go-build-.*matrix.lane.*inputs.source_ref \|\| github.sha/,
  );
});
