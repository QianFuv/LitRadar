/** Resolve the immutable source for a complete release or same-commit retry. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { pathToFileURL } from "node:url";
import {
  findRelease,
  github,
  validateReleaseIdentity,
} from "./release-github.mjs";
import { parseVersion } from "./release-version.mjs";

/** Read checked-out source facts and reject ancestry failures. */
function git(...args) {
  return execFileSync("git", args, { encoding: "utf8" });
}

/** Resolve the main commit without allowing an existing version to change ownership. */
export async function resolveReleaseContext(
  options,
  request = github,
  readGit = git,
) {
  assert.equal(
    options.ref,
    "refs/heads/main",
    "Release operations require main",
  );
  const outputs = {
    source: options.head,
    published: false,
    build: false,
  };
  assert.match(outputs.source, /^[a-f0-9]{40}$/);
  const version = parseVersion(options.version);
  const tag = `v${version}`;
  const release = await findRelease(tag, request);
  const reference = await request(`git/ref/tags/${tag}`);
  const tagged = reference ? await request(`commits/${tag}`) : null;
  const checkedVersion = readGit("show", `${outputs.source}:VERSION`);
  assert.equal(
    parseVersion(checkedVersion),
    version,
    "Requested version must match source VERSION",
  );
  outputs.published = validateReleaseIdentity(
    release,
    tagged?.sha,
    outputs.source,
  );
  outputs.build = !outputs.published;
  outputs.version = version;
  return outputs;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const outputs = await resolveReleaseContext({
    ref: process.env.GITHUB_REF,
    head: process.env.GITHUB_SHA,
    version: process.env.RELEASE_VERSION,
  });
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    Object.entries(outputs)
      .map(([key, value]) => `${key}=${value}\n`)
      .join(""),
  );
  console.log(outputs);
}
