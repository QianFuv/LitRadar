/** Emit a common CI summary and source/platform/checksum inventory without external writes. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const STAGES = ["detect-changes", "quality", "build-acceptance", "publish"];
const STATUSES = ["success", "failure", "cancelled", "skipped"];

/** Validate portable provenance and retain only check outcomes and numeric durations. */
function reportMetadata(metadata) {
  assert.match(metadata.sourceSha, /^[a-f0-9]{40}$/);
  assert(STAGES.includes(metadata.stage), "Unknown CI stage");
  assert(STATUSES.includes(metadata.status), "Unknown CI status");
  assert(
    metadata.platforms.length > 0 &&
      metadata.platforms.every((platform) =>
        ["linux/amd64", "windows/amd64", "independent"].includes(platform),
      ),
    "Unknown artifact platform",
  );
  return {
    schema: 1,
    sourceSha: metadata.sourceSha,
    platforms: metadata.platforms,
    stage: metadata.stage,
    status: metadata.status,
    checks: Object.entries(metadata.checks ?? {}).map(([name, check]) => ({
      name,
      status: check.outcome ?? check.result ?? "skipped",
      durationSeconds: /^\d+$/.test(check.outputs?.duration_seconds ?? "")
        ? Number(check.outputs.duration_seconds)
        : null,
    })),
  };
}

/** Resolve a relative report path without allowing inputs to escape their artifact root. */
function containedPath(root, relative) {
  const resolved = path.resolve(root, relative);
  const normalized = path.relative(root, resolved);
  assert(
    !normalized.startsWith("..") && !path.isAbsolute(normalized),
    "Report path escaped artifact root",
  );
  return resolved;
}

/** Reject linked ancestors before reading or writing a selected artifact path. */
async function checkAncestors(root, filename) {
  let current = root;
  for (const segment of [
    "",
    ...path.relative(root, filename).split(path.sep).filter(Boolean),
  ]) {
    current = path.join(current, segment);
    const entry = await fs.lstat(current).catch((error) => {
      if (error.code !== "ENOENT") throw error;
      return null;
    });
    if (!entry) return;
    assert(!entry.isSymbolicLink(), "Artifact report refuses symbolic links");
  }
}

/**
 * Inventory only selected artifact paths, streaming large files and preserving missing-path evidence.
 * @param {string} root - Artifact root.
 * @param {string[]} inputs - Relative files or directories included in the upload.
 * @param {object} metadata - Source SHA, platforms, stage, status and optional check outcomes.
 * @param {string} manifestName - Reserved output path relative to root.
 * @returns {Promise<object>} Written manifest, excluding its own checksum.
 */
export async function writeCiReport(
  root,
  inputs,
  metadata,
  manifestName = "ci-manifest.json",
) {
  const identity = reportMetadata(metadata);
  root = path.resolve(root);
  const manifestPath = containedPath(root, manifestName);
  await checkAncestors(root, manifestPath);
  const files = new Map();
  const missing = [];

  /** Inspect each selected path without following symbolic links or buffering entire images. */
  async function visit(filename) {
    if (filename === manifestPath) return;
    const entry = await fs.lstat(filename);
    assert(!entry.isSymbolicLink(), "Artifact report refuses symbolic links");
    if (entry.isDirectory()) {
      for (const child of (await fs.readdir(filename)).sort())
        if (metadata.includeHidden || !child.startsWith("."))
          await visit(path.join(filename, child));
    } else {
      assert(entry.isFile(), "Artifact report requires regular files");
      const sha256 = createHash("sha256");
      for await (const chunk of createReadStream(filename))
        sha256.update(chunk);
      const name = path.relative(root, filename).split(path.sep).join("/");
      files.set(name, {
        path: name,
        bytes: entry.size,
        sha256: sha256.digest("hex"),
      });
    }
  }

  for (const input of inputs) {
    const filename = containedPath(root, input);
    await checkAncestors(root, filename);
    const entry = await fs.lstat(filename).catch((error) => {
      if (error.code !== "ENOENT") throw error;
      return null;
    });
    if (entry) {
      const count = files.size;
      await visit(filename);
      if (files.size === count) missing.push(input);
    } else missing.push(input);
  }
  const report = {
    ...identity,
    missing,
    files: [...files.values()].sort((first, second) =>
      first.path < second.path ? -1 : first.path > second.path ? 1 : 0,
    ),
  };
  assert(
    !metadata.requireFiles ||
      metadata.status !== "success" ||
      (missing.length === 0 && files.size > 0),
    "Successful artifact is missing requested payloads",
  );
  await fs.mkdir(path.dirname(manifestPath), { recursive: true });
  const existing = await fs.lstat(manifestPath).catch((error) => {
    if (error.code !== "ENOENT") throw error;
    return null;
  });
  assert(!existing || existing.isFile(), "Unsafe report destination");
  await fs.writeFile(manifestPath, JSON.stringify(report, null, 2) + "\n");
  return report;
}

/** Render the same status/source/platform/check table for every workflow stage. */
export function ciSummary(report) {
  return [
    `## CI / ${report.stage}`,
    "",
    `- Status: ${report.status}`,
    `- Source: \`${report.sourceSha}\``,
    `- Platforms: ${report.platforms.join(", ")}`,
    `- Artifact files: ${report.files.length} (SHA-256); missing paths: ${report.missing.length}`,
    "",
    "| Check | Status | Duration (s) |",
    "| --- | --- | ---: |",
    ...report.checks.map(
      (check) =>
        `| ${check.name.replaceAll("|", "&#124;").replaceAll("\n", " ")} | ${check.status} | ${check.durationSeconds ?? "—"} |`,
    ),
    "",
  ].join("\n");
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [mode, ...inputs] = process.argv.slice(2);
  assert(["manifest", "summary"].includes(mode), "Unknown CI report mode");
  const metadata = {
    sourceSha: process.env.CI_SOURCE_SHA ?? process.env.GITHUB_SHA,
    platforms: process.env.CI_PLATFORMS.split(","),
    stage: process.env.CI_STAGE,
    status: process.env.CI_STATUS,
    checks: JSON.parse(process.env.CI_CHECKS ?? "{}"),
    requireFiles: process.env.CI_REQUIRE_FILES === "true",
    includeHidden: process.env.CI_INCLUDE_HIDDEN === "true",
  };
  if (mode === "manifest")
    await writeCiReport(
      process.env.CI_REPORT_ROOT,
      inputs,
      metadata,
      process.env.CI_REPORT_FILE,
    );
  else {
    const manifest = process.env.CI_REPORT_ROOT
      ? await fs
          .readFile(
            containedPath(
              path.resolve(process.env.CI_REPORT_ROOT),
              process.env.CI_REPORT_FILE ?? "ci-manifest.json",
            ),
            "utf8",
          )
          .then(JSON.parse)
          .catch((error) => {
            if (error.code !== "ENOENT") throw error;
            return null;
          })
      : null;
    const report = {
      files: [],
      missing: [],
      ...manifest,
      ...reportMetadata(metadata),
    };
    await fs.appendFile(process.env.GITHUB_STEP_SUMMARY, ciSummary(report));
  }
}
