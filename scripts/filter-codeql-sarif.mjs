/**
 * Filters exact, reviewed CodeQL test-fixture findings from SARIF output.
 */

import { readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const EXPECTED_SCHEMA_VERSION = 1;
const EXPECTED_RULE_ID = "rust/hard-coded-cryptographic-value";
const FINGERPRINT_PROPERTY = "primaryLocationLineHash";
const FINGERPRINT_PATTERN = /^[0-9a-f]{8,64}:[1-9][0-9]*$/u;

/**
 * Require a non-empty string field from parsed JSON.
 *
 * @param {unknown} value - Parsed field value.
 * @param {string} fieldName - Human-readable field name.
 * @returns {string} The validated string.
 */
function requireNonEmptyString(value, fieldName) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${fieldName} must be a non-empty string`);
  }
  return value;
}

/**
 * Build an unambiguous key for one reviewed SARIF result.
 *
 * @param {string} ruleId - CodeQL rule identifier.
 * @param {string} sourcePath - Repository-relative source path.
 * @param {string} fingerprint - CodeQL primary-location fingerprint.
 * @returns {string} A stable map key.
 */
function findingKey(ruleId, sourcePath, fingerprint) {
  return JSON.stringify([ruleId, sourcePath, fingerprint]);
}

/**
 * Parse and validate the exact reviewed-finding allowlist.
 *
 * @param {string} rawAllowlist - Raw JSON allowlist content.
 * @param {Date} today - Date used for expiry enforcement.
 * @returns {{ entries: Map<string, object>, metadata: object }} Validated entries and metadata.
 */
export function parseReviewedFindings(rawAllowlist, today = new Date()) {
  const parsed = JSON.parse(rawAllowlist);
  if (parsed.schemaVersion !== EXPECTED_SCHEMA_VERSION) {
    throw new Error(
      `unsupported allowlist schema version: ${parsed.schemaVersion}`,
    );
  }
  const ruleId = requireNonEmptyString(parsed.ruleId, "ruleId");
  if (ruleId !== EXPECTED_RULE_ID) {
    throw new Error(`allowlist ruleId must be ${EXPECTED_RULE_ID}`);
  }
  const owner = requireNonEmptyString(parsed.owner, "owner");
  const rationale = requireNonEmptyString(parsed.rationale, "rationale");
  const expiresOn = requireNonEmptyString(parsed.expiresOn, "expiresOn");
  if (!/^\d{4}-\d{2}-\d{2}$/u.test(expiresOn)) {
    throw new Error("expiresOn must use YYYY-MM-DD format");
  }
  const todayText = today.toISOString().slice(0, 10);
  if (expiresOn < todayText) {
    throw new Error(`reviewed finding allowlist expired on ${expiresOn}`);
  }
  if (!Array.isArray(parsed.findings) || parsed.findings.length === 0) {
    throw new Error("findings must be a non-empty array");
  }

  const entries = new Map();
  for (const finding of parsed.findings) {
    const sourcePath = requireNonEmptyString(finding.path, "findings[].path");
    if (
      path.posix.normalize(sourcePath) !== sourcePath ||
      sourcePath.startsWith("/")
    ) {
      throw new Error(
        `finding path must be normalized and repository-relative: ${sourcePath}`,
      );
    }
    const fingerprint = requireNonEmptyString(
      finding.fingerprint,
      "findings[].fingerprint",
    );
    if (!FINGERPRINT_PATTERN.test(fingerprint)) {
      throw new Error(`invalid CodeQL fingerprint: ${fingerprint}`);
    }
    const key = findingKey(ruleId, sourcePath, fingerprint);
    if (entries.has(key)) {
      throw new Error(
        `duplicate reviewed finding: ${sourcePath} ${fingerprint}`,
      );
    }
    entries.set(key, { ruleId, sourcePath, fingerprint });
  }

  return {
    entries,
    metadata: { expiresOn, owner, rationale, ruleId },
  };
}

/**
 * Recursively collect SARIF files from one path.
 *
 * @param {string} inputPath - SARIF file or directory.
 * @returns {Promise<string[]>} Sorted absolute SARIF file paths.
 */
async function collectSarifFiles(inputPath) {
  const inputMetadata = await stat(inputPath);
  if (inputMetadata.isFile()) {
    if (!inputPath.endsWith(".sarif")) {
      throw new Error(`expected a .sarif file: ${inputPath}`);
    }
    return [path.resolve(inputPath)];
  }
  if (!inputMetadata.isDirectory()) {
    throw new Error(
      `SARIF input is neither a file nor directory: ${inputPath}`,
    );
  }

  const discovered = [];

  /**
   * Visit one SARIF output directory.
   *
   * @param {string} directoryPath - Directory to inspect.
   * @returns {Promise<void>} Completes after all descendants are inspected.
   */
  async function visit(directoryPath) {
    const entries = await readdir(directoryPath, { withFileTypes: true });
    for (const entry of entries) {
      const entryPath = path.join(directoryPath, entry.name);
      if (entry.isDirectory()) {
        await visit(entryPath);
      } else if (entry.isFile() && entry.name.endsWith(".sarif")) {
        discovered.push(path.resolve(entryPath));
      }
    }
  }

  await visit(inputPath);
  discovered.sort();
  if (discovered.length === 0) {
    throw new Error(`no SARIF files found under ${inputPath}`);
  }
  return discovered;
}

/**
 * Extract an exact allowlist key from one SARIF result when possible.
 *
 * @param {object} result - SARIF result object.
 * @returns {string | undefined} Exact result key, or undefined for incomplete results.
 */
function resultKey(result) {
  const ruleId = result.ruleId;
  const sourcePath =
    result.locations?.[0]?.physicalLocation?.artifactLocation?.uri;
  const fingerprint = result.partialFingerprints?.[FINGERPRINT_PROPERTY];
  if (
    typeof ruleId !== "string" ||
    typeof sourcePath !== "string" ||
    typeof fingerprint !== "string"
  ) {
    return undefined;
  }
  return findingKey(ruleId, sourcePath, fingerprint);
}

/**
 * Remove only exact reviewed findings from one or more SARIF files.
 *
 * @param {{ sarifPath: string, allowlistPath: string, today?: Date }} options - Filter inputs.
 * @returns {Promise<{ reviewedCount: number, remainingCount: number, sarifFileCount: number, expiresOn: string }>} Filter summary.
 */
export async function filterReviewedFindings(options) {
  const today = options.today ?? new Date();
  const rawAllowlist = await readFile(options.allowlistPath, "utf8");
  const { entries, metadata } = parseReviewedFindings(rawAllowlist, today);
  const sarifFiles = await collectSarifFiles(options.sarifPath);
  const usage = new Map();
  const documents = [];
  let remainingCount = 0;

  for (const sarifFile of sarifFiles) {
    const document = JSON.parse(await readFile(sarifFile, "utf8"));
    if (!Array.isArray(document.runs)) {
      throw new Error(`SARIF document has no runs array: ${sarifFile}`);
    }
    for (const run of document.runs) {
      const results = Array.isArray(run.results) ? run.results : [];
      const retained = [];
      for (const result of results) {
        const key = resultKey(result);
        if (key !== undefined && entries.has(key)) {
          const nextUsage = (usage.get(key) ?? 0) + 1;
          if (nextUsage > 1) {
            throw new Error(`reviewed finding matched more than once: ${key}`);
          }
          usage.set(key, nextUsage);
        } else {
          retained.push(result);
          remainingCount += 1;
        }
      }
      run.results = retained;
    }
    documents.push({ document, sarifFile });
  }

  const unusedEntries = [];
  for (const [key, entry] of entries) {
    if (!usage.has(key)) {
      unusedEntries.push(`${entry.sourcePath} ${entry.fingerprint}`);
    }
  }
  if (unusedEntries.length > 0) {
    throw new Error(`unused reviewed findings: ${unusedEntries.join(", ")}`);
  }

  for (const { document, sarifFile } of documents) {
    await writeFile(sarifFile, `${JSON.stringify(document)}\n`, "utf8");
  }

  return {
    reviewedCount: usage.size,
    remainingCount,
    sarifFileCount: sarifFiles.length,
    expiresOn: metadata.expiresOn,
  };
}

const invokedPath = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href
  : "";
if (invokedPath === import.meta.url) {
  const [sarifPath, allowlistPath] = process.argv.slice(2);
  if (sarifPath === undefined || allowlistPath === undefined) {
    process.stderr.write(
      "usage: node scripts/filter-codeql-sarif.mjs <sarif-path> <allowlist-path>\n",
    );
    process.exitCode = 2;
  } else {
    try {
      const summary = await filterReviewedFindings({
        sarifPath,
        allowlistPath,
      });
      process.stdout.write(`${JSON.stringify(summary)}\n`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`CodeQL SARIF filter failed: ${message}\n`);
      process.exitCode = 1;
    }
  }
}
