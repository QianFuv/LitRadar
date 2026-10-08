/** Generate a complete, categorized commit list from published release history. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { compareVersions, parseVersion } from "./release-version.mjs";

const COMMIT_TYPES = new Map([
  ["feat", "Features"],
  ["fix", "Bug fixes"],
  ["perf", "Performance"],
  ["refactor", "Refactoring"],
  ["test", "Tests"],
  ["build", "Build"],
  ["ci", "CI"],
  ["docs", "Documentation"],
  ["style", "Style"],
  ["chore", "Maintenance"],
  ["revert", "Reverts"],
  ["other", "Other commits"],
]);

/** Escape commit subjects so Markdown and HTML cannot hide or alter list entries. */
function escapeSubject(subject) {
  return subject
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/([\\`*_{}\[\]()#!|~])/g, "\\$1");
}

/** List every commit since the highest reachable, older published stable version. */
export async function generateReleaseNotes(
  {
    version,
    commit,
    repository,
    serverUrl = "https://github.com",
    cwd = process.cwd(),
  },
  request,
) {
  parseVersion(version);
  assert.match(commit, /^[a-f0-9]{40}$/);
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  const git = (...args) =>
    execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    }).trim();
  assert.equal(
    git("rev-parse", "--is-shallow-repository"),
    "false",
    "Release notes require full Git history",
  );
  const reachableTags = new Set(git("tag", "--merged", commit).split(/\r?\n/));
  const releases = [];
  for (let page = 1; ; page++) {
    const batch = await request(`releases?per_page=100&page=${page}`);
    assert(Array.isArray(batch), "Cannot list releases for commit history");
    releases.push(...batch);
    if (batch.length < 100) break;
  }
  const previous = releases
    .filter(
      (release) =>
        !release.draft &&
        !release.prerelease &&
        /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(release.tag_name) &&
        reachableTags.has(release.tag_name) &&
        compareVersions(release.tag_name.slice(1), version) < 0,
    )
    .sort((first, second) =>
      compareVersions(second.tag_name.slice(1), first.tag_name.slice(1)),
    )[0];
  const range = previous ? `${previous.tag_name}..${commit}` : commit;
  const log = git("log", "--reverse", "--format=%H%x00%s", range, "--");
  const commits = log
    ? log.split(/\r?\n/).map((line) => {
        const separator = line.indexOf("\0");
        const hash = line.slice(0, separator);
        assert.match(hash, /^[a-f0-9]{40}$/);
        return { hash, subject: line.slice(separator + 1) };
      })
    : [];
  const baseUrl = `${serverUrl.replace(/\/$/, "")}/${repository}`;
  const groups = new Map([...COMMIT_TYPES.keys()].map((type) => [type, []]));
  for (const { hash, subject } of commits) {
    const type = /^([a-z]+)(?:\([^\r\n)]+\))?!?:\s+/.exec(subject)?.[1];
    groups
      .get(COMMIT_TYPES.has(type) ? type : "other")
      .push(
        `- ${escapeSubject(subject)} ([${hash.slice(0, 7)}](${baseUrl}/commit/${hash}))`,
      );
  }
  const sections = [
    `## Commits (${commits.length})`,
    previous
      ? `Changes since ${previous.tag_name}.`
      : "All commits in this first stable release.",
  ];
  for (const [type, entries] of groups) {
    if (entries.length)
      sections.push(
        `### ${COMMIT_TYPES.get(type)} (${entries.length})\n\n${entries.join("\n")}`,
      );
  }
  const comparison = previous
    ? `compare/${previous.tag_name}...v${version}`
    : `commits/v${version}`;
  sections.push(
    `**Full changelog:** [${previous ? `${previous.tag_name}...v${version}` : `v${version}`}](${baseUrl}/${comparison})`,
  );
  const notes = sections.join("\n\n") + "\n";
  assert(
    Buffer.byteLength(notes, "utf8") <= 125000,
    "Complete commit list exceeds the release body limit",
  );
  return notes;
}
