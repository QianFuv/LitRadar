/** Stage a verified static export for compilation into the production executable. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { buildCspManifest } from "./generate-csp.mjs";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const COMPRESSED_EXTENSIONS = new Set([
  ".css",
  ".html",
  ".js",
  ".json",
  ".map",
  ".svg",
  ".txt",
  ".xml",
]);

/**
 * Copy regular export files while rebuilding deterministic compressed representations.
 * @param {string} source - Validated export directory.
 * @param {string} destination - Task-owned staging directory.
 * @returns {Promise<void>} Completion of the recursive copy.
 */
async function copyExport(source, destination) {
  await fs.mkdir(destination, { recursive: true });
  for (const entry of await fs.readdir(source, { withFileTypes: true })) {
    assert(
      !entry.isSymbolicLink(),
      "Static export must not contain symbolic links",
    );
    const input = path.join(source, entry.name);
    const output = path.join(destination, entry.name);
    if (entry.isDirectory()) await copyExport(input, output);
    else {
      assert(entry.isFile(), "Static export contains a nonregular asset");
      if (entry.name.endsWith(".gz")) continue;
      const data = await fs.readFile(input);
      await fs.writeFile(output, data);
      if (COMPRESSED_EXTENSIONS.has(path.extname(entry.name))) {
        await fs.writeFile(`${output}.gz`, gzipSync(data, { level: 9 }));
      }
    }
  }
}

/**
 * Stage a complete export only after its CSP manifest matches the actual HTML.
 * @param {string} projectRoot - Repository whose dedicated embed output may be replaced.
 * @returns {Promise<string>} Absolute generated embed directory.
 */
export async function stageWebAssets(projectRoot = ROOT) {
  const root = await fs.realpath(projectRoot);
  const source = path.join(root, "app", "out");
  const destination = path.join(root, "internal", "webassets", "export");
  const expected = await buildCspManifest(source);
  const manifestPath = path.join(source, "csp-hashes.json");
  const metadata = await fs.lstat(manifestPath);
  assert(
    metadata.isFile() && metadata.size <= 4 * 1024 * 1024,
    "Invalid CSP manifest file",
  );
  const manifest = await fs.readFile(manifestPath, "utf8");
  assert.equal(
    manifest,
    `${JSON.stringify(expected, null, 2)}\n`,
    "CSP manifest does not match the canonical static export",
  );
  for (const filename of ["index.html", "404.html"]) {
    assert(
      (await fs.lstat(path.join(source, filename))).isFile(),
      `Missing production ${filename}`,
    );
  }
  await fs.mkdir(path.dirname(destination), { recursive: true });
  const parent = await fs.realpath(path.dirname(destination));
  assert.equal(
    parent,
    path.join(root, "internal", "webassets"),
    "Embed directory escaped repository",
  );
  const current = await fs.lstat(destination).catch((error) => {
    if (error.code !== "ENOENT") throw error;
    return null;
  });
  assert(
    !current || (current.isDirectory() && !current.isSymbolicLink()),
    "Unsafe embed destination",
  );
  await fs.rm(destination, { recursive: true, force: true });
  await copyExport(source, destination);
  assert.deepEqual(
    await buildCspManifest(destination),
    expected,
    "Staged HTML changed during copy",
  );
  return destination;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await stageWebAssets(process.argv[2] ?? ROOT);
}
