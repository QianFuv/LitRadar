/** Transfer the exact tested frontend export with source-bound, fail-closed integrity checks. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { buildCspManifest } from "./generate-csp.mjs";

/**
 * Inventory regular assets without following links; paths are portable and relative.
 * @param {string} directory - Export root to inspect.
 * @returns {Promise<Array<{path: string, sha256: string}>>} Sorted file identities.
 */
async function inventory(directory) {
  const metadata = await fs.lstat(directory);
  assert(
    metadata.isDirectory() && !metadata.isSymbolicLink(),
    "Unsafe export directory",
  );
  const files = [];
  for (const name of (await fs.readdir(directory)).sort()) {
    assert(!name.includes("\\"), "Nonportable export path");
    const filename = path.join(directory, name);
    const entry = await fs.lstat(filename);
    assert(!entry.isSymbolicLink(), "Export must not contain symbolic links");
    if (entry.isDirectory()) {
      for (const file of await inventory(filename))
        files.push({ ...file, path: `${name}/${file.path}` });
    } else {
      assert(entry.isFile(), "Export must contain only regular files");
      files.push({
        path: name,
        sha256: createHash("sha256")
          .update(await fs.readFile(filename))
          .digest("hex"),
      });
    }
  }
  return files;
}

/**
 * Copy a tested embed export into a new artifact, preserving its compressed bytes.
 * @param {string} sourceDirectory - Export embedded by the successful full-stack test.
 * @param {string} bundle - New task-owned artifact directory.
 * @param {string} source - Exact tested Git commit.
 * @returns {Promise<void>} Completion after verifying copied bytes.
 */
export async function prepareReleaseWeb(sourceDirectory, bundle, source) {
  assert.match(source, /^[a-f0-9]{40}$/);
  const files = await inventory(sourceDirectory);
  for (const required of ["index.html", "404.html", "csp-hashes.json"]) {
    assert(
      files.some((file) => file.path === required),
      `Missing tested ${required}`,
    );
  }
  await fs.mkdir(bundle);
  await fs.cp(sourceDirectory, path.join(bundle, "app/out"), {
    recursive: true,
    errorOnExist: true,
    force: false,
  });
  await fs.writeFile(
    path.join(bundle, "manifest.json"),
    JSON.stringify({ schema: 1, source, files }, null, 2) + "\n",
  );
  await verifyReleaseWeb(bundle, source);
}

/**
 * Require an exact source and file inventory before release consumers can use an export.
 * @param {string} bundle - Downloaded same-run artifact directory.
 * @param {string} source - Release context's expected Git commit.
 * @returns {Promise<string>} Verified export path for compilation.
 */
export async function verifyReleaseWeb(bundle, source) {
  assert.match(source, /^[a-f0-9]{40}$/);
  for (const directory of [bundle, path.join(bundle, "app")]) {
    const metadata = await fs.lstat(directory);
    assert(
      metadata.isDirectory() && !metadata.isSymbolicLink(),
      "Unsafe artifact directory",
    );
  }
  const manifestPath = path.join(bundle, "manifest.json");
  assert((await fs.lstat(manifestPath)).isFile(), "Invalid export manifest");
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  assert.equal(manifest.schema, 1, "Unsupported export manifest");
  assert.equal(
    manifest.source,
    source,
    "Frontend source differs from release source",
  );
  const output = path.join(bundle, "app/out");
  assert.deepEqual(
    await inventory(output),
    manifest.files,
    "Frontend file inventory differs",
  );
  for (const filename of ["index.html", "404.html", "csp-hashes.json"]) {
    assert(
      (await fs.lstat(path.join(output, filename))).isFile(),
      `Missing ${filename}`,
    );
  }
  assert.equal(
    await fs.readFile(path.join(output, "csp-hashes.json"), "utf8"),
    JSON.stringify(await buildCspManifest(output), null, 2) + "\n",
    "Frontend CSP manifest differs from HTML",
  );
  return output;
}

/**
 * Stage verified bytes without platform-dependent recompression and recheck the embed inventory.
 * @param {string} bundle - Same-run artifact directory.
 * @param {string} source - Expected release commit.
 * @param {string} projectRoot - Clean release source checkout.
 * @returns {Promise<string>} Final verified embed directory.
 */
export async function stageReleaseWeb(bundle, source, projectRoot) {
  const output = await verifyReleaseWeb(bundle, source);
  const expected = await inventory(output);
  const root = await fs.realpath(projectRoot);
  const parent = path.join(root, "internal/webassets");
  await fs.mkdir(parent, { recursive: true });
  assert.equal(
    await fs.realpath(parent),
    parent,
    "Embed directory escaped repository",
  );
  const destination = path.join(parent, "export");
  await fs.mkdir(destination);
  for (const name of await fs.readdir(output)) {
    await fs.cp(path.join(output, name), path.join(destination, name), {
      recursive: true,
      errorOnExist: true,
      force: false,
    });
  }
  assert.deepEqual(
    await inventory(destination),
    expected,
    "Final embedded export differs from tested bytes",
  );
  return destination;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [mode, source] = process.argv.slice(2);
  const bundle = path.resolve("release-results/web");
  assert(
    ["prepare", "verify"].includes(mode),
    "Unknown frontend artifact mode",
  );
  if (mode === "prepare") {
    await fs.mkdir(path.dirname(bundle), { recursive: true });
    await prepareReleaseWeb("internal/webassets/export", bundle, source);
  } else {
    await verifyReleaseWeb(bundle, source);
  }
}
