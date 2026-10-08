/** Verify the complete locally replaced dependency trees and reproduce their bounded patches. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
const WORKSPACE_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
/** Hash exact dependency source bytes. */
function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

const policies = {
  "go-sdk": {
    commit: "3f3b699b2b67e1ed033a63d6651671dab53c2d32",
    archive: "b2d9bd11290bbf59552b6493d6db01b182c751828e6bdd5afa507cef0c852f20",
    changed: ["mcp/streamable.go", "mcp/mcp_test.go"],
    added: ["mcp/litradar_compat.go", "mcp/litradar_compat_test.go"],
  },
  "go-sqlite3": {
    commit: "b0be46fa28d17ee0b65c79774ac0dad84b6db068",
    archive: "1cbfe55076c554a634bb4901a858a08091703c1e2219354414c041f9db39eed8",
    changed: ["sqlite3.go"],
    added: ["sqlite3_litradar_nofollow_test.go"],
  },
};

/**
 * List every file, including hidden files, while rejecting links in a dependency tree.
 * @param {string} root - Directory to inspect.
 * @param {string} relative - Nested relative path.
 * @returns {Promise<string[]>} Sorted file paths.
 */
async function files(root, relative = "") {
  const result = [];
  for (const entry of await fs.readdir(path.join(root, relative), {
    withFileTypes: true,
  })) {
    const name = relative ? `${relative}/${entry.name}` : entry.name;
    assert(!entry.isSymbolicLink(), `Linked dependency path: ${name}`);
    if (entry.isDirectory()) result.push(...(await files(root, name)));
    else {
      assert(entry.isFile(), `Unexpected dependency node: ${name}`);
      result.push(name);
    }
  }
  return result.sort();
}
/** Consume one hunk with exact context, syntax and final-empty-line admission. */
function consumeUnifiedHunk(input, lines, index, cursor, output) {
  let removed = 0,
    added = 0;
  while (index + 1 < lines.length && !lines[index + 1].startsWith("@@")) {
    const line = lines[++index];
    if (line === "") {
      assert(index === lines.length - 1, "Empty patch line");
      break;
    }
    const prefix = line[0],
      value = line.slice(1);
    assert(" +-".includes(prefix), "Unsupported patch syntax");
    if (prefix !== "+") {
      assert.equal(input[cursor++], value, "Patch context mismatch");
      removed++;
    }
    if (prefix !== "-") {
      output.push(value);
      added++;
    }
  }
  return { index, cursor, removed, added };
}

/** Acquire the hash-pinned archive, validating downloads before exclusive cache publication. */
async function verifiedDependencyArchive(name, policy, upstream) {
  const archive = path.join(
    WORKSPACE_ROOT,
    "output/dependencies/upstream",
    `${name}.tar.gz`,
  );
  try {
    await fs.access(archive);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    const repository =
      name === "go-sdk" ? "modelcontextprotocol/go-sdk" : "mattn/go-sqlite3";
    const url = `https://codeload.github.com/${repository}/tar.gz/${policy.commit}`;
    assert.equal(upstream.archiveUrl, url);
    const response = await fetch(url, { signal: AbortSignal.timeout(60000) });
    assert(response.ok, `Upstream archive fetch failed: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    assert.equal(digest(bytes), policy.archive, "Downloaded archive changed");
    await fs.mkdir(path.dirname(archive), { recursive: true });
    await fs.writeFile(archive, bytes, { flag: "wx" });
  }
  assert.equal(
    digest(await fs.readFile(archive)),
    policy.archive,
    "Archive changed",
  );

  return archive;
}

/** Validate upstream inventory and bytes before local inventory and unchanged-source hashes. */
async function verifyDependencyInventory(temporary, upstream, local, policy) {
  assert.deepEqual(
    await files(temporary),
    upstream.files.map((file) => file.path).sort(),
    "Upstream manifest incomplete",
  );
  const upstreamMap = new Map(upstream.files.map((file) => [file.path, file]));
  for (const file of upstream.files)
    assert.equal(
      digest(await fs.readFile(path.join(temporary, file.path))),
      file.sha256,
      `Upstream changed: ${file.path}`,
    );
  const expectedNames = [...upstreamMap.keys(), ...policy.added].sort();
  assert.deepEqual(await files(local), expectedNames, "Unlisted local source");
  const changed = [...policy.changed, ...policy.added].sort();
  for (const file of upstream.files)
    if (!policy.changed.includes(file.path))
      assert.equal(
        digest(await fs.readFile(path.join(local, file.path))),
        file.sha256,
        `Out-of-scope source change: ${file.path}`,
      );

  return { upstreamMap, expectedNames, changed };
}

/** Reproduce each bounded change and publish patch bytes before their identity metadata. */
async function generateDependencyPatchRecords(
  policy,
  local,
  records,
  temporary,
  upstreamMap,
  changed,
) {
  const patches = [];
  for (const filename of changed) {
    const original = upstreamMap.has(filename)
      ? await fs.readFile(path.join(temporary, filename), "utf8")
      : "";
    const updated = await fs.readFile(path.join(local, filename), "utf8");
    let patch;
    if (original === "") {
      assert(updated.endsWith("\n"));
      const lines = updated.slice(0, -1).split("\n");
      patch = `diff --git a/${filename} b/${filename}\nnew file mode 100644\n--- /dev/null\n+++ b/${filename}\n@@ -0,0 +1,${lines.length} @@\n${lines.map((line) => "+" + line).join("\n")}\n`;
    } else {
      const diff = spawnSync(
        "git",
        [
          "diff",
          "--no-index",
          "--no-ext-diff",
          "--text",
          path.join(temporary, filename),
          path.join(local, filename),
        ],
        {
          encoding: "utf8",
          shell: false,
          timeout: 30_000,
          maxBuffer: 8 * 1024 * 1024,
        },
      );
      assert.ifError(diff.error);
      assert.equal(diff.status, 1, diff.stderr);
      patch = diff.stdout
        .replace(/^diff --git .*$/m, `diff --git a/${filename} b/${filename}`)
        .replace(/^--- .*$/m, `--- a/${filename}`)
        .replace(/^\+\+\+ .*$/m, `+++ b/${filename}`);
    }
    assert.equal(
      applyUnified(original, patch),
      updated,
      `Patch does not reproduce ${filename}`,
    );
    patches.push({
      path: filename,
      sha256: digest(Buffer.from(updated)),
      diff: patch,
    });
  }
  const patch = patches.map((entry) => entry.diff).join("");
  await fs.writeFile(path.join(records, "compatibility.patch"), patch);
  await fs.writeFile(
    path.join(records, "patched.json"),
    JSON.stringify(
      {
        upstreamCommit: policy.commit,
        patchSha256: digest(Buffer.from(patch)),
        files: patches.map(({ path, sha256 }) => ({ path, sha256 })),
      },
      null,
      2,
    ) + "\n",
  );
}

/** Validate recorded patch order and exact reproduction before local source hashes. */
async function verifyDependencyPatchRecords(
  policy,
  records,
  temporary,
  local,
  upstreamMap,
  changed,
) {
  const identity = JSON.parse(
    await fs.readFile(path.join(records, "patched.json"), "utf8"),
  );
  const patch = await fs.readFile(
    path.join(records, "compatibility.patch"),
    "utf8",
  );
  assert.equal(identity.upstreamCommit, policy.commit);
  assert.equal(digest(Buffer.from(patch)), identity.patchSha256);
  assert.deepEqual(
    identity.files.map((file) => file.path),
    changed,
  );
  const pieces = patch.split(/(?=^diff --git )/m).filter(Boolean);
  assert.equal(pieces.length, changed.length);
  for (const [index, file] of identity.files.entries()) {
    assert(
      pieces[index].startsWith(`diff --git a/${file.path} b/${file.path}\n`),
      "Wrong patch path",
    );
    const original = upstreamMap.has(file.path)
      ? await fs.readFile(path.join(temporary, file.path), "utf8")
      : "";
    assert.equal(
      digest(Buffer.from(applyUnified(original, pieces[index]))),
      file.sha256,
      "Reproduction failed",
    );
    assert.equal(
      digest(await fs.readFile(path.join(local, file.path))),
      file.sha256,
      "Patched local source changed",
    );
  }

  return identity;
}

/**
 * Apply one standard unified patch strictly to original bytes without invoking Git mutations.
 * @param {string} original - Original UTF-8 text with LF line endings.
 * @param {string} patch - One-file unified diff.
 * @returns {string} Reconstructed text.
 */
export function applyUnified(original, patch) {
  assert(!original.includes("\r"), "Unexpected upstream line endings");
  assert(
    original === "" || original.endsWith("\n"),
    "Missing original final newline",
  );
  const input = original === "" ? [] : original.slice(0, -1).split("\n");
  const output = [];
  const lines = patch.split("\n");
  let cursor = 0,
    sawHunk = false;
  for (let index = 0; index < lines.length; index++) {
    if (!lines[index].startsWith("@@")) continue;
    const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(
      lines[index],
    );
    assert(match, "Invalid patch hunk");
    sawHunk = true;
    const oldStart = +match[1] === 0 ? 0 : +match[1] - 1;
    const oldCount = match[2] === undefined ? 1 : +match[2];
    const newCount = match[4] === undefined ? 1 : +match[4];
    assert(oldStart >= cursor && oldStart <= input.length, "Overlapping patch");
    output.push(...input.slice(cursor, oldStart));
    cursor = oldStart;
    const hunk = consumeUnifiedHunk(input, lines, index, cursor, output);
    cursor = hunk.cursor;
    assert.equal(hunk.removed, oldCount, "Wrong removed count");
    assert.equal(hunk.added, newCount, "Wrong added count");
    index = hunk.index - 1;
  }
  assert(sawHunk, "Empty patch");
  output.push(...input.slice(cursor));
  return output.length ? output.join("\n") + "\n" : "";
}

/**
 * Reproduce a bounded dependency patch against its hash-pinned archive.
 * @param {string} name - Approved local dependency name.
 * @param {boolean} generate - Explicitly refresh task-owned patch records during development.
 * @returns {Promise<object>} Verified identity report.
 */
export async function verifyDependency(name, generate = false) {
  const policy = policies[name];
  assert(policy, "Unknown dependency");
  const local = path.join(WORKSPACE_ROOT, "third_party", name);
  const records = path.join(WORKSPACE_ROOT, "third_party", `${name}-patches`);
  const upstream = JSON.parse(
    await fs.readFile(path.join(records, "upstream.json"), "utf8"),
  );
  assert.equal(upstream.commit, policy.commit);
  assert.equal(upstream.archiveSha256, policy.archive);
  const archive = await verifiedDependencyArchive(name, policy, upstream);
  const temporary = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-dependency-"),
  );
  try {
    const extraction = spawnSync(
      "tar",
      ["-xf", archive, "-C", temporary, "--strip-components=1"],
      { encoding: "utf8", shell: false, timeout: 60_000 },
    );
    assert.ifError(extraction.error);
    assert.equal(extraction.status, 0, extraction.stderr);
    const { upstreamMap, expectedNames, changed } =
      await verifyDependencyInventory(temporary, upstream, local, policy);
    if (generate) {
      await generateDependencyPatchRecords(
        policy,
        local,
        records,
        temporary,
        upstreamMap,
        changed,
      );
    }
    const identity = await verifyDependencyPatchRecords(
      policy,
      records,
      temporary,
      local,
      upstreamMap,
      changed,
    );
    const report = {
      dependency: name,
      upstreamCommit: policy.commit,
      upstreamFiles: upstream.files.length,
      localFiles: expectedNames.length,
      patchSha256: identity.patchSha256,
      licenseSha256: digest(await fs.readFile(path.join(local, "LICENSE"))),
      result: "Passed",
    };
    await fs.mkdir(path.join(WORKSPACE_ROOT, "output/dependencies"), {
      recursive: true,
    });
    await fs.writeFile(
      path.join(
        WORKSPACE_ROOT,
        "output/dependencies",
        `${name}-integrity.json`,
      ),
      JSON.stringify(report, null, 2) + "\n",
    );
    return report;
  } finally {
    await fs.rm(temporary, { recursive: true, force: true });
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  assert.equal(process.argv.length, 3, "Specify go-sdk or go-sqlite3");
  console.log(await verifyDependency(process.argv[2]));
}
