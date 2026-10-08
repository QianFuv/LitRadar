/** Verify release triggers and retry ownership without publishing anything. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createHash } from "node:crypto";
import {
  releaseAssetNames,
  validateAssets,
} from "../scripts/release-assets.mjs";
import { resolveReleaseContext } from "../scripts/release-context.mjs";
import { generateReleaseNotes } from "../scripts/release-notes.mjs";
import { verifyReleaseJobs } from "../scripts/release-status.mjs";
import { verifyReleaseEvidence } from "../scripts/release-evidence.mjs";
import {
  detectVersion,
  detectRelease,
  parseVersion,
  versionChange,
  compareVersions,
} from "../scripts/release-version.mjs";
import {
  findRelease,
  latestRelease,
  validateReleaseIdentity,
} from "../scripts/release-github.mjs";

const RELEASE_COMMIT = "a".repeat(40);
const RELEASE_ASSETS = [
  "litradar_0.2.0_linux_amd64.tar.gz",
  "litradar_0.2.0_windows_amd64.zip",
  "SHA256SUMS",
].map((name) => ({ name, digest: `sha256:${"b".repeat(64)}`, size: 100 }));

test("complete releases require exactly Linux amd64, Windows x64 and one checksum manifest", () => {
  assert.deepEqual(releaseAssetNames("0.2.1"), [
    "litradar_0.2.1_linux_amd64.tar.gz",
    "litradar_0.2.1_windows_amd64.zip",
    "SHA256SUMS",
  ]);
});

test("release completion rejects skipped publication even when earlier jobs succeeded", () => {
  const jobs = {
    context: { result: "success", outputs: { published: "false" } },
    windows: { result: "success" },
    linux: { result: "success" },
    publish: { result: "success" },
    promote: { result: "success" },
  };
  verifyReleaseJobs(jobs);
  for (const name of Object.keys(jobs)) {
    for (const result of ["skipped", "failure", "cancelled"]) {
      assert.throws(() =>
        verifyReleaseJobs({ ...jobs, [name]: { ...jobs[name], result } }),
      );
    }
  }
  const retry = {
    ...jobs,
    context: { result: "success", outputs: { published: "true" } },
    windows: { result: "skipped" },
    linux: { result: "skipped" },
    publish: { result: "skipped" },
  };
  verifyReleaseJobs(retry);
  assert.throws(() =>
    verifyReleaseJobs({ ...retry, promote: { result: "skipped" } }),
  );
  assert.throws(() =>
    verifyReleaseJobs({ ...retry, windows: { result: "success" } }),
  );
  assert.throws(() =>
    verifyReleaseJobs({ ...jobs, context: { result: "success", outputs: {} } }),
  );
});

test("publication accepts only the image bytes and source verified by platform smoke tests", () => {
  const image = {
    Id: `sha256:${"b".repeat(64)}`,
    Os: "linux",
    Architecture: "amd64",
    Config: {
      Labels: {
        "org.opencontainers.image.revision": RELEASE_COMMIT,
        "org.opencontainers.image.version": "0.2.1",
      },
    },
  };
  const evidence = {
    image,
    loaded: structuredClone(image),
    linux: { status: "passed", imageId: image.Id },
    windows: {
      status: "passed",
      sourceCommit: RELEASE_COMMIT,
      version: "0.2.1",
    },
    commit: RELEASE_COMMIT,
    version: "0.2.1",
  };
  verifyReleaseEvidence(evidence);
  for (const modified of [
    { loaded: { ...image, Id: `sha256:${"c".repeat(64)}` } },
    { loaded: { ...image, Architecture: "arm64" } },
    { linux: { ...evidence.linux, status: "failed" } },
    { windows: { ...evidence.windows, status: "failed" } },
    { windows: { ...evidence.windows, sourceCommit: "c".repeat(40) } },
    { windows: { ...evidence.windows, version: "0.2.0" } },
    {
      image: {
        ...image,
        Config: {
          Labels: {
            ...image.Config.Labels,
            "org.opencontainers.image.revision": "c".repeat(40),
          },
        },
      },
    },
  ])
    assert.throws(() => verifyReleaseEvidence({ ...evidence, ...modified }));
  for (const key of ["image", "loaded"]) {
    const modified = structuredClone(evidence);
    modified[key].Config.Labels["org.opencontainers.image.version"] = "0.2.0";
    assert.throws(
      () => verifyReleaseEvidence(modified),
      /Linux version differs/,
    );
  }
});

test("release context permits only the current main version and immutable same-commit retries", async () => {
  const options = {
    ref: "refs/heads/main",
    head: RELEASE_COMMIT,
    version: "0.2.1",
  };
  const readGit = (...args) => {
    assert.deepEqual(args, ["show", `${RELEASE_COMMIT}:VERSION`]);
    return "0.2.1\n";
  };
  const absent = async (resource) =>
    resource.startsWith("releases?") ? [] : null;
  assert.deepEqual(await resolveReleaseContext(options, absent, readGit), {
    source: RELEASE_COMMIT,
    version: "0.2.1",
    published: false,
    build: true,
  });
  const published = async (resource) => {
    if (resource.startsWith("releases/tags/")) return { draft: false };
    if (resource.startsWith("git/ref/")) return { ref: "refs/tags/v0.2.1" };
    if (resource.startsWith("commits/")) return { sha: RELEASE_COMMIT };
    throw new Error(`Unexpected resource ${resource}`);
  };
  assert.equal(
    (await resolveReleaseContext(options, published, readGit)).build,
    false,
  );
  await assert.rejects(
    resolveReleaseContext(
      { ...options, ref: "refs/heads/topic" },
      absent,
      readGit,
    ),
    /require main/,
  );
  await assert.rejects(
    resolveReleaseContext(options, absent, () => "0.2.0"),
    /match source VERSION/,
  );
  await assert.rejects(
    resolveReleaseContext(
      options,
      async (resource) =>
        resource.startsWith("commits/")
          ? { sha: "c".repeat(40) }
          : published(resource),
      readGit,
    ),
    /another commit/,
  );
});

test("final release verification rejects absent assets, prereleases and missing tags", async () => {
  const release = {
    draft: false,
    prerelease: false,
    assets: RELEASE_ASSETS,
    target_commitish: RELEASE_COMMIT,
  };
  await createReleaseHarness("verify", releaseReplies(release)).main();
  for (const invalid of [
    { ...release, draft: true },
    { ...release, prerelease: true },
    { ...release, assets: RELEASE_ASSETS.slice(0, 1) },
    { ...release, assets: [...RELEASE_ASSETS, { name: "extra.sha256" }] },
    {
      ...release,
      assets: RELEASE_ASSETS.map((asset) => ({ ...asset, size: 0 })),
    },
  ])
    await assert.rejects(
      createReleaseHarness("verify", releaseReplies(invalid)).main(),
    );
  await assert.rejects(
    createReleaseHarness("verify", {
      "releases/tags/v0.2.0": [release],
      "git/ref/tags/v0.2.0": [null],
    }).main(),
    /tag must match/,
  );
});

/** Return controlled HTTP metadata without permitting any real network operation. */
function releaseMetadataResponse(reply, events) {
  const status = reply?.httpStatus ?? (reply === null ? 404 : 200);
  /** Observe decoding so absent and failed HTTP responses cannot silently read bodies. */
  async function decodeMetadata() {
    events.push(["decode", status]);
    if (reply?.jsonError) throw new SyntaxError(reply.jsonError);
    return reply;
  }
  return { status, ok: status >= 200 && status < 300, json: decodeMetadata };
}

/** Execute the real CLI functions with every external boundary replaced by a local mock. */
function createReleaseHarness(
  mode,
  replies,
  {
    assets = RELEASE_ASSETS,
    failCommand = null,
    notesError = null,
    envOverrides = {},
  } = {},
) {
  const events = [];
  const environment = {
    GH_TOKEN: "fixture-token",
    GITHUB_REPOSITORY: "Fixture/Repository",
    GITHUB_API_URL: "https://api.example",
    RELEASE_VERSION: "0.2.0",
    RELEASE_SOURCE_SHA: RELEASE_COMMIT,
    GITHUB_OUTPUT: "fixture-output",
    ...envOverrides,
  };
  /** Consume only explicitly supplied metadata responses and inspect authorization. */
  async function fetchMetadata(url, options) {
    const prefix = "https://api.example/repos/Fixture/Repository/";
    assert(url.startsWith(prefix));
    assert.deepEqual(options.headers, {
      Authorization: "Bearer fixture-token",
      Accept: "application/vnd.github+json",
    });
    assert(options.signal instanceof AbortSignal);
    const resource = url.slice(prefix.length);
    events.push(["request", resource]);
    const queue = replies[resource];
    assert(queue?.length, `Unexpected metadata request: ${resource}`);
    return releaseMetadataResponse(queue.shift(), events);
  }
  /** Record commands without spawning a process or publishing release state. */
  function executeCommand(command, args, options) {
    assert.deepEqual(options, { stdio: "inherit", timeout: 300000 });
    events.push(["command", command, ...args]);
    if (failCommand && failCommand(command, args))
      throw new Error("Fixture command failed");
  }
  /** Record exact check output instead of writing a workflow output file. */
  function appendOutput(filename, value) {
    assert.equal(filename, "fixture-output");
    events.push(["output", value]);
  }
  /** Capture the generated body before any draft creation or update. */
  function writeNotes(filename, value) {
    assert.equal(filename, "release-results/notes.md");
    assert.equal(value, "fixture release notes\n");
    events.push(["notes", value]);
  }
  /** Isolate history generation while checking immutable source inputs. */
  async function generateFixtureNotes(options, request) {
    assert.equal(options.version, "0.2.0");
    assert.equal(options.commit, RELEASE_COMMIT);
    assert.equal(options.repository, "Fixture/Repository");
    assert.equal(typeof request, "function");
    if (notesError) throw new Error(notesError);
    return "fixture release notes\n";
  }
  /** Observe validation before release commands while supplying fixture-verified asset metadata. */
  function validateFixtureAssets(directory, version, windowsOnly = false) {
    assert.equal(version, "0.2.0");
    events.push(["validate", directory, windowsOnly]);
    return assets;
  }
  const source = fs
    .readFileSync(
      new URL("../scripts/release-github.mjs", import.meta.url),
      "utf8",
    )
    .replace(/^import .+;\r?$/gm, "")
    .replace(/^export /gm, "");
  const entry = source.lastIndexOf("\nif (process.argv[1]");
  assert(
    entry > 0,
    "CLI entry must be isolated before executing mocked functions",
  );
  const load = new Function(
    "assert",
    "execFileSync",
    "fs",
    "path",
    "parseVersion",
    "releaseAssetNames",
    "validateAssets",
    "compareVersions",
    "generateReleaseNotes",
    "process",
    "fetch",
    source.slice(0, entry) + "\nreturn {main, github};",
  );
  const functions = load(
    assert,
    executeCommand,
    {
      appendFileSync: appendOutput,
      writeFileSync: writeNotes,
    },
    path,
    parseVersion,
    releaseAssetNames,
    validateFixtureAssets,
    compareVersions,
    generateFixtureNotes,
    { argv: ["node", "fixture", mode], env: environment },
    fetchMetadata,
  );
  return { ...functions, events };
}

/** Supply the ordinary release and tag-reference lookup responses independently. */
function releaseReplies(release, updated = release) {
  return {
    "releases/tags/v0.2.0": [release, updated],
    "git/ref/tags/v0.2.0": [{ ref: "refs/tags/v0.2.0" }],
    "commits/v0.2.0": [{ sha: RELEASE_COMMIT }],
  };
}

/** Select command events without using production dispatch decisions as expectations. */
function releaseCommands(harness) {
  return harness.events.filter((event) => event[0] === "command");
}

/** Verify immutable identity lookup precedes check output and published-release skipping. */
async function checksAndSkipsPublishedReleases() {
  const release = {
    id: 7,
    draft: false,
    target_commitish: RELEASE_COMMIT,
    assets: [],
  };
  for (const mode of ["check", "prepare", "publish"]) {
    const harness = createReleaseHarness(mode, releaseReplies(release));
    await harness.main();
    assert.deepEqual(
      harness.events
        .filter((event) => event[0] === "request")
        .map((event) => event[1]),
      ["releases/tags/v0.2.0", "git/ref/tags/v0.2.0", "commits/v0.2.0"],
    );
    assert.deepEqual(harness.events.slice(0, 2), [
      ["request", "releases/tags/v0.2.0"],
      ["request", "git/ref/tags/v0.2.0"],
    ]);
    assert.deepEqual(releaseCommands(harness), []);
    assert.deepEqual(
      harness.events.filter((event) => event[0] === "output"),
      mode === "check" ? [["output", "published=true\n"]] : [],
    );
  }
}

/** Verify draft creation follows asset validation and preserves exact upload arguments. */
async function preparesAbsentRelease() {
  const harness = createReleaseHarness("prepare", {
    "releases/tags/v0.2.0": [null],
    "git/ref/tags/v0.2.0": [null],
    "releases?per_page=100&page=1": [[]],
  });
  await harness.main();
  assert.deepEqual(releaseCommands(harness), [
    [
      "command",
      "gh",
      "release",
      "create",
      "v0.2.0",
      "--target",
      RELEASE_COMMIT,
      "--title",
      "LitRadar v0.2.0",
      "--draft",
      "--notes-file",
      "release-results/notes.md",
    ],
    [
      "command",
      "gh",
      "release",
      "upload",
      "v0.2.0",
      ...RELEASE_ASSETS.map((asset) =>
        path.join("release-results/assets", asset.name),
      ),
      "--clobber",
    ],
  ]);
  const validation = harness.events.findIndex(
    (event) => event[0] === "validate",
  );
  const command = harness.events.findIndex((event) => event[0] === "command");
  assert(validation >= 0 && validation < command);
  assert(harness.events.findIndex((event) => event[0] === "notes") < command);
  assert(!harness.events.some((event) => event[1] === "commits/v0.2.0"));
}

test("draft retries refresh categorized notes and note failures prevent publication writes", async () => {
  const draft = { id: 7, draft: true, target_commitish: RELEASE_COMMIT };
  const harness = createReleaseHarness("prepare", releaseReplies(draft));
  await harness.main();
  assert.deepEqual(releaseCommands(harness)[0], [
    "command",
    "gh",
    "release",
    "edit",
    "v0.2.0",
    "--notes-file",
    "release-results/notes.md",
  ]);
  const failure = createReleaseHarness("prepare", releaseReplies(draft), {
    notesError: "History unavailable",
  });
  await assert.rejects(failure.main(), /History unavailable/);
  assert.deepEqual(releaseCommands(failure), []);
});

test("release notes include every commit once, classify types and exclude unpublished baselines", async (context) => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "litradar-notes-test-"),
  );
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: directory,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  git("init");
  /** Create an isolated history fixture without changing workspace Git state. */
  function commit(subject) {
    git(
      "-c",
      "user.name=Release Test",
      "-c",
      "user.email=release-test@example.invalid",
      "-c",
      "commit.gpgsign=false",
      "commit",
      "--allow-empty",
      "-m",
      subject,
    );
    return git("rev-parse", "HEAD");
  }
  const baseline = commit("chore: initial version");
  git("tag", "v0.1.0");
  git("tag", "v0.1.1");
  const entries = [
    ["feat(api)!: change endpoint", "Features"],
    ["fix: escape <script>, [link](url), and ~~literal~~", "Bug fixes"],
    ["perf(search): reduce allocations", "Performance"],
    ["refactor(api): simplify dispatch", "Refactoring"],
    ["test: cover retries", "Tests"],
    ["build: reuse cache", "Build"],
    ["ci: publish artifacts", "CI"],
    ["docs: explain releases", "Documentation"],
    ["style: format scripts", "Style"],
    ["chore: bump version", "Maintenance"],
    ["revert: undo change", "Reverts"],
    ["Merge branch 'topic'", "Other commits"],
    ["custom(scope): keep unrecognized commits", "Other commits"],
  ];
  const hashes = entries.map(([subject]) => commit(subject));
  git("tag", "v0.1.2");
  git("tag", "v0.1.3");
  git("tag", "v0.2.0");
  const options = {
    version: "0.2.0",
    commit: hashes.at(-1),
    repository: "Fixture/Repository",
    cwd: directory,
  };
  const requests = [];
  const notes = await generateReleaseNotes(options, async (resource) => {
    requests.push(resource);
    if (resource.endsWith("page=1"))
      return Array.from({ length: 100 }, () => ({
        tag_name: "v0.1.2",
        draft: true,
      }));
    return [
      { tag_name: "v0.1.0" },
      { tag_name: "v0.1.1" },
      { tag_name: "v0.1.3", prerelease: true },
      { tag_name: "v0.2.0" },
      { tag_name: "v0.1.9" },
    ];
  });
  assert.equal(requests.length, 2);
  assert(notes.includes("Changes since v0.1.1."));
  assert(notes.includes(`## Commits (${entries.length})`));
  assert(!notes.includes(baseline));
  for (let index = 0; index < entries.length; index++) {
    const section = notes
      .split(`### ${entries[index][1]} (`)[1]
      ?.split("\n### ")[0];
    assert(section?.includes(`/commit/${hashes[index]})`));
    assert.equal(notes.split(`/commit/${hashes[index]})`).length, 2);
  }
  assert(notes.includes("&lt;script&gt;"));
  assert(notes.includes("\\[link\\]\\(url\\)"));
  assert(notes.includes("\\~\\~literal\\~\\~"));
  assert(notes.includes("/compare/v0.1.1...v0.2.0"));
  const initial = await generateReleaseNotes(options, async () => []);
  assert(initial.includes(`## Commits (${entries.length + 1})`));
  assert(initial.includes(`/commit/${baseline})`));
  assert(initial.includes("/commits/v0.2.0"));
  await assert.rejects(
    generateReleaseNotes(options, async () => null),
    /Cannot list releases/,
  );
});

/** Verify publication requires a draft and does not change the latest pointer. */
async function publishesOnlyPreparedDraft() {
  const harness = createReleaseHarness(
    "publish",
    releaseReplies({ id: 7, draft: true, target_commitish: RELEASE_COMMIT }),
  );
  await harness.main();
  assert.deepEqual(releaseCommands(harness), [
    [
      "command",
      "gh",
      "release",
      "edit",
      "v0.2.0",
      "--draft=false",
      "--latest=false",
    ],
  ]);
  const absent = createReleaseHarness("publish", {
    "releases/tags/v0.2.0": [null],
    "git/ref/tags/v0.2.0": [null],
    "releases?per_page=100&page=1": [[]],
  });
  await assert.rejects(absent.main(), /Prepare the release before publishing/);
  assert.deepEqual(releaseCommands(absent), []);
}

/** Stop release actions immediately when a mocked upload or draft command fails. */
async function stopsAfterReleaseCommandFailure() {
  const harness = createReleaseHarness(
    "prepare",
    {
      "releases/tags/v0.2.0": [null],
      "git/ref/tags/v0.2.0": [null],
      "releases?per_page=100&page=1": [[]],
    },
    { failCommand: () => true },
  );
  await assert.rejects(harness.main(), /Fixture command failed/);
  assert.equal(releaseCommands(harness).length, 1);
  assert.equal(releaseCommands(harness)[0][3], "create");
}

/** Distinguish absent, failed and invalid-JSON metadata without decoding HTTP errors. */
async function preservesMetadataFailures() {
  for (const status of [404, 403, 500]) {
    const harness = createReleaseHarness("check", {
      fixture: [{ httpStatus: status }],
    });
    if (status === 404) assert.equal(await harness.github("fixture"), null);
    else
      await assert.rejects(
        harness.github("fixture"),
        new RegExp(`GitHub metadata request failed: ${status}`),
      );
    assert(!harness.events.some((event) => event[0] === "decode"));
  }
  const invalid = createReleaseHarness("check", {
    fixture: [{ jsonError: "invalid fixture JSON" }],
  });
  await assert.rejects(invalid.github("fixture"), /invalid fixture JSON/);
}

/** Promote the highest stable version without requiring source-version inputs. */
async function promotesLatestInOrder() {
  const releases = [
    { tag_name: "v0.2.0", draft: false, prerelease: false },
    { tag_name: "v0.3.0", draft: false, prerelease: false },
  ];
  const envOverrides = {
    RELEASE_VERSION: undefined,
    RELEASE_SOURCE_SHA: undefined,
    GITHUB_SHA: undefined,
  };
  const harness = createReleaseHarness(
    "promote",
    { "releases?per_page=100&page=1": [releases] },
    { envOverrides },
  );
  await harness.main();
  assert.deepEqual(releaseCommands(harness), [
    [
      "command",
      "docker",
      "buildx",
      "imagetools",
      "create",
      "--tag",
      "ghcr.io/fixture/repository:latest",
      "ghcr.io/fixture/repository:v0.3.0",
    ],
    ["command", "gh", "release", "edit", "v0.3.0", "--latest"],
  ]);
  const failed = createReleaseHarness(
    "promote",
    { "releases?per_page=100&page=1": [releases] },
    { envOverrides, failCommand: () => true },
  );
  await assert.rejects(failed.main(), /Fixture command failed/);
  assert.equal(releaseCommands(failed).length, 1);
}

test(
  "check and published-release skip preserve identity lookup order",
  checksAndSkipsPublishedReleases,
);
test(
  "prepare validates assets before exact draft commands",
  preparesAbsentRelease,
);
test(
  "publish requires an existing draft and preserves latest",
  publishesOnlyPreparedDraft,
);
test(
  "release command failure stops subsequent actions",
  stopsAfterReleaseCommandFailure,
);
test(
  "metadata failures preserve HTTP and JSON distinctions",
  preservesMetadataFailures,
);
test(
  "promotion updates Docker before GitHub without source-version inputs",
  promotesLatestInOrder,
);

test("draft lookup survives a missing tag endpoint and preserves ownership", async () => {
  const commit = "a".repeat(40);
  const draft = { tag_name: "v0.2.0", draft: true, target_commitish: commit };
  const requests = [];
  const found = await findRelease("v0.2.0", async (resource) => {
    requests.push(resource);
    if (resource === "releases/tags/v0.2.0") return null;
    if (resource === "releases?per_page=100&page=1")
      return Array.from({ length: 100 }, () => ({ tag_name: "v0.1.0" }));
    assert.equal(resource, "releases?per_page=100&page=2");
    return [draft];
  });
  assert.deepEqual(found, draft);
  assert.deepEqual(requests, [
    "releases/tags/v0.2.0",
    "releases?per_page=100&page=1",
    "releases?per_page=100&page=2",
  ]);
  assert.equal(validateReleaseIdentity(found, null, commit), false);
  assert.throws(
    () => validateReleaseIdentity(found, null, "b".repeat(40)),
    /another commit/,
  );
});

test("release lookup distinguishes published, absent and inaccessible releases", async () => {
  const published = { tag_name: "v0.2.0", draft: false };
  assert.equal(
    await findRelease("v0.2.0", async (resource) => {
      assert.equal(resource, "releases/tags/v0.2.0");
      return published;
    }),
    published,
  );
  assert.equal(
    await findRelease("v0.2.0", async (resource) =>
      resource.startsWith("releases/tags/") ? null : [],
    ),
    null,
  );
  await assert.rejects(
    findRelease("v0.2.0", async (resource) => {
      if (resource.startsWith("releases/tags/")) return null;
      throw new Error("GitHub metadata request failed: 403");
    }),
    /403/,
  );
  await assert.rejects(
    findRelease("v0.2.0", async () => null),
    /Cannot list releases/,
  );
});

test("the checked-in release version is valid", () => {
  parseVersion(fs.readFileSync(new URL("../VERSION", import.meta.url), "utf8"));
});

test("a release requires verified Linux and Windows archives before publication", (context) => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "litradar-assets-test-"),
  );
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const archives = releaseAssetNames("0.2.0").slice(0, -1);
  const lines = archives.map((name) => {
    const bytes = Buffer.from(`tested ${name}`);
    fs.writeFileSync(path.join(directory, name), bytes);
    return `${createHash("sha256").update(bytes).digest("hex")}  ${name}\n`;
  });
  const checksum = path.join(directory, "SHA256SUMS");
  fs.writeFileSync(checksum, lines.slice(0, 1).join(""));
  assert.throws(() => validateAssets(directory, "0.2.0"), /expected platforms/);
  fs.writeFileSync(checksum, lines.join(""));
  assert.equal(validateAssets(directory, "0.2.0").length, 3);
  fs.appendFileSync(checksum, lines[0]);
  assert.throws(() => validateAssets(directory, "0.2.0"), /Duplicate/);
  fs.writeFileSync(checksum, lines.join(""));
  fs.appendFileSync(path.join(directory, archives[1]), "changed bytes");
  assert.throws(() => validateAssets(directory, "0.2.0"), /Checksum differs/);
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
  const retry = {
    event: "workflow_dispatch",
    after: pushed,
    ref: "refs/heads/main",
    version: "0.2.0",
  };
  assert.deepEqual(detectRelease(retry, directory), {
    version: "0.2.0",
    tag: "v0.2.0",
    release: true,
  });
  assert.equal(
    detectRelease({ event: "push", before: bumped, after: pushed }, directory)
      .release,
    false,
  );
  assert.equal(
    detectRelease(
      { event: "push", before: established, after: pushed },
      directory,
    ).release,
    true,
  );
  for (const invalid of [
    { version: "0.1.0" },
    { version: "0.3.0" },
    { version: "" },
    { version: "v0.2.0" },
    { ref: "refs/heads/develop" },
    { ref: "refs/tags/v0.2.0" },
    { after: "main" },
    { event: "pull_request" },
  ])
    assert.throws(() => detectRelease({ ...retry, ...invalid }, directory));
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
