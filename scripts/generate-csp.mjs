/**
 * Generate a deterministic CSP hash manifest for a static HTML export.
 *
 * @module generate-csp
 */

import { createHash } from "node:crypto";
import { readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const MANIFEST_FILENAME = "csp-hashes.json";
const MANIFEST_VERSION = 1;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

/**
 * Return a CSP-compatible SHA-256 source expression.
 *
 * @param {string | Uint8Array} value - Exact script text or file bytes.
 * @returns {string} SHA-256 source expression using standard Base64.
 */
export function sha256Source(value) {
  return `sha256-${createHash("sha256").update(value).digest("base64")}`;
}

/**
 * Find the closing angle bracket of an opening tag while respecting quoted attributes.
 *
 * @param {string} html - Complete HTML document.
 * @param {number} startIndex - Index immediately after the opening tag name.
 * @returns {number} Index of the closing angle bracket.
 */
function findOpeningTagEnd(html, startIndex) {
  let quote = null;
  for (let index = startIndex; index < html.length; index += 1) {
    const character = html[index];
    if (quote !== null) {
      if (character === quote) {
        quote = null;
      }
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (character === ">") {
      return index;
    }
  }
  throw new Error("Static HTML contains an unterminated script opening tag");
}

/**
 * Return whether a script opening tag contains a real src attribute.
 *
 * @param {string} openingTag - Exact script opening tag including angle brackets.
 * @returns {boolean} True when the tag contains a src attribute.
 */
function hasSrcAttribute(openingTag) {
  let index = "<script".length;
  while (index < openingTag.length - 1) {
    while (/\s|\//u.test(openingTag[index] ?? "")) {
      index += 1;
    }
    const nameStart = index;
    while (!/[\s=/>]/u.test(openingTag[index] ?? ">")) {
      index += 1;
    }
    if (index === nameStart) {
      index += 1;
      continue;
    }
    const name = openingTag.slice(nameStart, index);
    if (name.toLowerCase() === "src") {
      return true;
    }
    while (/\s/u.test(openingTag[index] ?? "")) {
      index += 1;
    }
    if (openingTag[index] !== "=") {
      continue;
    }
    index += 1;
    while (/\s/u.test(openingTag[index] ?? "")) {
      index += 1;
    }
    const quote = openingTag[index];
    if (quote === '"' || quote === "'") {
      index += 1;
      while (index < openingTag.length && openingTag[index] !== quote) {
        index += 1;
      }
      index += 1;
    } else {
      while (!/[\s>]/u.test(openingTag[index] ?? ">")) {
        index += 1;
      }
    }
  }
  return false;
}

/**
 * Extract CSP hashes for every inline script in document order.
 *
 * @param {string} html - UTF-8 HTML document.
 * @returns {string[]} Ordered SHA-256 source expressions.
 */
export function extractInlineScriptHashes(html) {
  const hashes = [];
  const openingPattern = /<script(?=[\s/>])/giu;
  let openingMatch;
  while ((openingMatch = openingPattern.exec(html)) !== null) {
    const openingEnd = findOpeningTagEnd(html, openingPattern.lastIndex);
    const closingPattern = /<\/script\s*>/giu;
    closingPattern.lastIndex = openingEnd + 1;
    const closingMatch = closingPattern.exec(html);
    if (closingMatch === null) {
      throw new Error("Static HTML contains an unterminated script element");
    }
    const openingTag = html.slice(openingMatch.index, openingEnd + 1);
    if (!hasSrcAttribute(openingTag)) {
      hashes.push(sha256Source(html.slice(openingEnd + 1, closingMatch.index)));
    }
    openingPattern.lastIndex = closingPattern.lastIndex;
  }
  return hashes;
}

/**
 * Recursively collect regular HTML files while rejecting symbolic links.
 *
 * @param {string} rootDirectory - Static export root.
 * @param {string} currentDirectory - Directory currently being traversed.
 * @returns {Promise<string[]>} Sorted absolute HTML paths.
 */
async function collectHtmlFiles(
  rootDirectory,
  currentDirectory = rootDirectory,
) {
  const entries = await readdir(currentDirectory, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(currentDirectory, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error("Static export must not contain symbolic links");
    }
    if (entry.isDirectory()) {
      files.push(...(await collectHtmlFiles(rootDirectory, entryPath)));
    } else if (entry.isFile() && entry.name.toLowerCase().endsWith(".html")) {
      files.push(entryPath);
    }
  }
  return files;
}

/**
 * Build the deterministic manifest object for one static export.
 *
 * @param {string} outputDirectory - Static export root.
 * @returns {Promise<object>} Serializable CSP manifest.
 */
export async function buildCspManifest(outputDirectory) {
  const rootDirectory = path.resolve(outputDirectory);
  const htmlFiles = await collectHtmlFiles(rootDirectory);
  htmlFiles.sort((left, right) =>
    Buffer.compare(
      Buffer.from(path.relative(rootDirectory, left), "utf8"),
      Buffer.from(path.relative(rootDirectory, right), "utf8"),
    ),
  );
  if (htmlFiles.length === 0) {
    throw new Error("Static export contains no HTML files");
  }
  const files = [];
  const uniqueScriptHashes = new Set();
  for (const htmlPath of htmlFiles) {
    const bytes = await readFile(htmlPath);
    const html = UTF8_DECODER.decode(bytes);
    const inlineScriptHashes = extractInlineScriptHashes(html);
    for (const hash of inlineScriptHashes) {
      uniqueScriptHashes.add(hash);
    }
    files.push({
      path: path.relative(rootDirectory, htmlPath).split(path.sep).join("/"),
      html_sha256: sha256Source(bytes),
      inline_script_hashes: inlineScriptHashes,
    });
  }
  return {
    version: MANIFEST_VERSION,
    algorithm: "sha256",
    files,
    script_hashes: [...uniqueScriptHashes].sort(),
  };
}

/**
 * Generate csp-hashes.json under one static export directory.
 *
 * @param {string} outputDirectory - Static export root.
 * @returns {Promise<string>} Absolute manifest path.
 */
export async function generateCspManifest(outputDirectory) {
  const rootDirectory = path.resolve(outputDirectory);
  const manifest = await buildCspManifest(rootDirectory);
  const manifestPath = path.join(rootDirectory, MANIFEST_FILENAME);
  await writeFile(
    manifestPath,
    `${JSON.stringify(manifest, null, 2)}\n`,
    "utf8",
  );
  return manifestPath;
}

const isMainModule =
  process.argv[1] !== undefined &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMainModule) {
  const outputDirectory = process.argv[2] ?? "out";
  generateCspManifest(outputDirectory)
    .then((manifestPath) => {
      process.stdout.write(`Generated ${manifestPath}\n`);
    })
    .catch((error) => {
      const message =
        error instanceof Error
          ? error.message
          : "Unknown CSP generation failure";
      process.stderr.write(`CSP manifest generation failed: ${message}\n`);
      process.exitCode = 1;
    });
}
