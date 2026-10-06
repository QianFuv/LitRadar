/** Check release ownership and prepare or publish a retryable GitHub release. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { compareVersions, parseVersion } from "./release-version.mjs";

/** Select the highest published stable version, independent of completion order. */
export function latestRelease(releases) {
  return releases
    .filter(
      (release) =>
        !release.draft &&
        !release.prerelease &&
        /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(release.tag_name),
    )
    .sort((first, second) =>
      compareVersions(second.tag_name.slice(1), first.tag_name.slice(1)),
    )[0];
}

/** Promote the highest public version while the workflow holds the latest-update lock. */
async function promoteLatest() {
  const releases = [];
  for (let page = 1; ; page++) {
    const batch = await github(`releases?per_page=100&page=${page}`);
    assert(Array.isArray(batch), "Cannot list published releases");
    releases.push(...batch);
    if (batch.length < 100) break;
  }
  const release = latestRelease(releases);
  assert(release, "No published stable release to promote");
  const image = `ghcr.io/${process.env.GITHUB_REPOSITORY.toLowerCase()}`;
  execFileSync(
    "docker",
    [
      "buildx",
      "imagetools",
      "create",
      "--tag",
      `${image}:latest`,
      `${image}:${release.tag_name}`,
    ],
    { stdio: "inherit", timeout: 300000 },
  );
  execFileSync("gh", ["release", "edit", release.tag_name, "--latest"], {
    stdio: "inherit",
    timeout: 300000,
  });
}

/** Reject reused tags and report whether the exact commit has already been published. */
export function validateReleaseIdentity(release, taggedCommit, commit) {
  assert(
    !taggedCommit || taggedCommit === commit,
    "Release tag belongs to another commit",
  );
  assert(
    !release || taggedCommit || release.target_commitish === commit,
    "Draft release belongs to another commit",
  );
  return Boolean(release && !release.draft);
}

/** Find an exact release tag, including drafts omitted by the tag endpoint. */
export async function findRelease(tag, request = github) {
  const release = await request(`releases/tags/${tag}`);
  if (release) return release;
  for (let page = 1; ; page++) {
    const batch = await request(`releases?per_page=100&page=${page}`);
    assert(Array.isArray(batch), "Cannot list releases for draft lookup");
    const draft = batch.find((candidate) => candidate.tag_name === tag);
    if (draft) return draft;
    if (batch.length < 100) return null;
  }
}

/** Read GitHub metadata, distinguishing absence from authentication and service errors. */
async function github(resource) {
  const response = await fetch(
    `${process.env.GITHUB_API_URL ?? "https://api.github.com"}/repos/${process.env.GITHUB_REPOSITORY}/${resource}`,
    {
      headers: {
        Authorization: `Bearer ${process.env.GH_TOKEN}`,
        Accept: "application/vnd.github+json",
      },
      signal: AbortSignal.timeout(30000),
    },
  );
  if (response.status === 404) return null;
  assert(response.ok, `GitHub metadata request failed: ${response.status}`);
  return response.json();
}

/** Run release commands only for this version and immutable commit identity. */
async function main() {
  const mode = process.argv[2];
  assert(["check", "prepare", "publish", "promote"].includes(mode));
  assert(process.env.GH_TOKEN && process.env.GITHUB_REPOSITORY);
  if (mode === "promote") return promoteLatest();
  const version = parseVersion(process.env.RELEASE_VERSION);
  const tag = `v${version}`;
  const commit = process.env.GITHUB_SHA;
  assert.match(commit, /^[a-f0-9]{40}$/);
  assert(process.env.GH_TOKEN && process.env.GITHUB_REPOSITORY);
  const [release, reference] = await Promise.all([
    findRelease(tag),
    github(`git/ref/tags/${tag}`),
  ]);
  const tagged = reference ? await github(`commits/${tag}`) : null;
  const published = validateReleaseIdentity(release, tagged?.sha, commit);
  if (mode === "check") {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `published=${published}\n`);
    return;
  }
  if (published) return;
  const gh = (...args) =>
    execFileSync("gh", args, { stdio: "inherit", timeout: 300000 });
  if (mode === "prepare") {
    const directory = "release-results/assets";
    const files = fs
      .readdirSync(directory)
      .filter((file) => file.endsWith(".tar.gz") || file === "SHA256SUMS");
    assert.equal(
      files.length,
      3,
      "Both architecture archives and checksums are required",
    );
    for (const filename of [
      "SHA256SUMS",
      `litradar_${version}_linux_amd64.tar.gz`,
      `litradar_${version}_linux_arm64.tar.gz`,
    ]) {
      assert(files.includes(filename), `Missing release asset: ${filename}`);
    }
    if (!release) {
      gh(
        "release",
        "create",
        tag,
        "--target",
        commit,
        "--title",
        `LitRadar ${tag}`,
        "--draft",
        "--generate-notes",
      );
    }
    gh(
      "release",
      "upload",
      tag,
      ...files.map((file) => path.join(directory, file)),
      "--clobber",
    );
  } else {
    assert(release?.draft, "Prepare the release before publishing");
    gh("release", "edit", tag, "--draft=false", "--latest=false");
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  await main();
