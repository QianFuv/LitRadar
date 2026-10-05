/** Verify release triggers and retry ownership without publishing anything. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  detectVersion,
  parseVersion,
  versionChange,
} from "../scripts/release-version.mjs";
import {
  latestRelease,
  validateReleaseIdentity,
} from "../scripts/release-github.mjs";

test("the checked-in release version is valid", () => {
  parseVersion(fs.readFileSync(new URL("../VERSION", import.meta.url), "utf8"));
});

test("an older release finishing later cannot move latest backwards", () => {
  const releases = [
    { tag_name: "v1.9.0", draft: false },
    { tag_name: "v1.10.0", draft: false },
    { tag_name: "v2.0.0", draft: true },
    { tag_name: "v3.0.0", prerelease: true },
    { tag_name: "v4.0.0-beta" },
  ];
  assert.equal(latestRelease(releases).tag_name, "v1.10.0");
  assert.equal(latestRelease(releases.toReversed()).tag_name, "v1.10.0");
});

test("ordinary pushes and formatting changes never publish", () => {
  assert.equal(versionChange("0.1.0", "0.1.0\n").release, false);
  assert.equal(versionChange("0.1.0", "0.1.1").release, true);
  assert.equal(versionChange("0.9.9", "0.10.0").release, true);
  assert.equal(versionChange("1.9.9", "2.0.0").tag, "v2.0.0");
  assert.throws(() => versionChange("1.2.3", "1.2.2"), /increase/);
  for (const version of [
    "1.2",
    "v1.2.3",
    "01.2.3",
    "1.2.3-beta",
    "1.2.3\nrelease=true",
  ]) {
    assert.throws(() => parseVersion(version));
  }
});

test("version detection includes a bump before the last commit of a push", (context) => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "litradar-release-test-"),
  );
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: directory,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  git("init");
  fs.mkdirSync(path.join(directory, "app"));
  fs.writeFileSync(
    path.join(directory, "app/package.json"),
    '{"version":"0.1.0"}',
  );
  const commit = () => {
    git("add", "--all");
    git(
      "-c",
      "user.name=Release Test",
      "-c",
      "user.email=release-test@example.invalid",
      "-c",
      "commit.gpgsign=false",
      "commit",
      "-m",
      "test",
    );
    return git("rev-parse", "HEAD");
  };
  const baseline = commit();
  fs.writeFileSync(path.join(directory, "VERSION"), "0.1.0\n");
  const established = commit();
  assert.equal(detectVersion(baseline, established, directory).release, false);
  fs.writeFileSync(path.join(directory, "VERSION"), "0.2.0\n");
  const bumped = commit();
  fs.writeFileSync(path.join(directory, "README.md"), "ordinary change\n");
  const pushed = commit();
  assert.equal(detectVersion(established, pushed, directory).release, true);
  assert.equal(detectVersion(bumped, pushed, directory).release, false);
  assert.throws(
    () => detectVersion("0".repeat(40), pushed, directory),
    /baseline/,
  );
});

test("reruns resume only the same draft or skip the same published release", () => {
  const commit = "a".repeat(40);
  const other = "b".repeat(40);
  assert.equal(validateReleaseIdentity(null, null, commit), false);
  assert.equal(
    validateReleaseIdentity(
      { draft: true, target_commitish: commit },
      null,
      commit,
    ),
    false,
  );
  assert.equal(validateReleaseIdentity({ draft: false }, commit, commit), true);
  assert.throws(
    () => validateReleaseIdentity(null, other, commit),
    /another commit/,
  );
  assert.throws(
    () =>
      validateReleaseIdentity(
        { draft: true, target_commitish: other },
        null,
        commit,
      ),
    /another commit/,
  );
});
