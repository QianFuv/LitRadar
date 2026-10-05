/** Detect a release from the version change across the entire pushed commit range. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { pathToFileURL } from "node:url";

/** Validate stable numeric versions used in archive names, tags and image tags. */
export function parseVersion(value) {
  const version = value.trim();
  assert.match(
    version,
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/,
    "VERSION must contain a stable MAJOR.MINOR.PATCH version",
  );
  return version;
}

/** Require an increase and ignore unchanged versions, including whitespace-only edits. */
export function versionChange(previous, current) {
  const version = parseVersion(current);
  const difference = compareVersions(version, previous);
  assert(difference >= 0, "VERSION must increase");
  return { version, tag: `v${version}`, release: difference > 0 };
}

/** Compare numeric version components without lexical or integer-overflow errors. */
export function compareVersions(first, second) {
  const oldParts = parseVersion(second).split(".").map(BigInt);
  const newParts = parseVersion(first).split(".").map(BigInt);
  const difference = newParts.findIndex(
    (part, index) => part !== oldParts[index],
  );
  return difference < 0
    ? 0
    : newParts[difference] > oldParts[difference]
      ? 1
      : -1;
}

/** Compare pushed tips; the old frontend version supplies the pre-VERSION baseline. */
export function detectVersion(before, after, cwd = process.cwd()) {
  assert.match(before, /^[a-f0-9]{40}$/);
  assert.match(after, /^[a-f0-9]{40}$/);
  assert(
    !/^0+$/.test(before),
    "A version release requires an existing branch baseline",
  );
  const git = (...args) =>
    execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
  const hasVersion =
    git("ls-tree", "--name-only", before, "--", "VERSION") === "VERSION";
  const previous = hasVersion
    ? git("show", `${before}:VERSION`)
    : JSON.parse(git("show", `${before}:app/package.json`)).version;
  return versionChange(previous, git("show", `${after}:VERSION`));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const result = detectVersion(process.env.BEFORE_SHA, process.env.GITHUB_SHA);
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    Object.entries(result)
      .map(([key, value]) => `${key}=${value}\n`)
      .join(""),
  );
  console.log(result);
}
