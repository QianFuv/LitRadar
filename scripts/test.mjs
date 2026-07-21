/**
 * Run the five repository test layers with cross-platform failure and signal propagation.
 */

import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WORKSPACE_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const APP_ROOT = path.join(WORKSPACE_ROOT, "app");
const VALID_MODES = new Set([
  "fast",
  "integration",
  "e2e-smoke",
  "all",
  "diagnostics",
]);
const WINDOWS_SIGNAL_EXIT_CODES = { SIGINT: 130, SIGTERM: 143 };

let activeChild;
let receivedSignal;

/**
 * Create one executable step.
 *
 * @param {string} label - Human-readable layer label.
 * @param {string} command - Executable name.
 * @param {string[]} args - Executable arguments.
 * @param {string} [cwd=WORKSPACE_ROOT] - Working directory.
 * @param {NodeJS.ProcessEnv} [env={}] - Step-specific environment.
 * @returns {{label: string, command: string, args: string[], cwd: string, env: NodeJS.ProcessEnv}} Step definition.
 */
function step(label, command, args, cwd = WORKSPACE_ROOT, env = {}) {
  return { label, command, args, cwd, env };
}

/**
 * Create one pnpm-backed step rooted in the frontend application.
 *
 * @param {string} label - Human-readable layer label.
 * @param {string[]} args - pnpm arguments.
 * @param {NodeJS.ProcessEnv} [env={}] - Step-specific environment.
 * @returns {{label: string, command: string, args: string[], cwd: string, env: NodeJS.ProcessEnv}} Step definition.
 */
function pnpmStep(label, args, env = {}) {
  return step(label, "pnpm", args, APP_ROOT, env);
}

/**
 * Return the Rust and jsdom quick-feedback steps.
 *
 * @param {boolean} isCi - Whether CI reporters are enabled.
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function fastSteps(isCi) {
  return [
    step("Rust library and binary tests", "cargo", [
      "test",
      "--workspace",
      "--lib",
      "--bins",
      "--locked",
    ]),
    pnpmStep(
      "Vitest jsdom",
      ["test:unit"],
      isCi ? { LITRADAR_VITEST_JUNIT: "./test-results/vitest/junit.xml" } : {},
    ),
  ];
}

/**
 * Return the workspace integration and contract steps.
 *
 * @param {boolean} isCi - Whether the nextest CI profile is selected.
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function integrationSteps(isCi) {
  const nextestArgs = ["nextest", "run", "--workspace", "--locked"];
  if (isCi) {
    nextestArgs.push("--profile", "ci");
  }
  return [
    step("Rust workspace through nextest", "cargo", nextestArgs),
    step("Rust doctests", "cargo", [
      "test",
      "--workspace",
      "--doc",
      "--locked",
    ]),
    pnpmStep("Generated OpenAPI idempotence", ["generate:api:check"]),
    pnpmStep("Shared frontend API contract", [
      "exec",
      "vitest",
      "run",
      "--config",
      "vitest.config.tsx",
      "tests/api-contract.test.tsx",
      "--project",
      "unit-jsdom",
    ]),
  ];
}

/**
 * Return the real-backend browser smoke step.
 *
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function e2eSmokeSteps() {
  return [pnpmStep("Real-backend Chromium smoke", ["test:e2e:full-stack"])];
}

/**
 * Return every static, test, browser, and build step.
 *
 * @param {boolean} isCi - Whether CI reporters and the nextest CI profile are selected.
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function allSteps(isCi) {
  return [
    step("Rust formatting", "cargo", ["fmt", "--all", "--", "--check"]),
    step("Rust dependency ordering", "cargo", [
      "sort",
      "--workspace",
      "--check",
    ]),
    step("Rust Clippy", "cargo", [
      "clippy",
      "--workspace",
      "--all-targets",
      "--all-features",
      "--locked",
      "--",
      "-D",
      "warnings",
    ]),
    pnpmStep("Frontend lint", ["lint"]),
    pnpmStep("Frontend formatting", ["format:check"]),
    pnpmStep("Frontend type checking", ["exec", "tsc", "--noEmit"]),
    ...integrationSteps(isCi),
    pnpmStep(
      "Vitest jsdom",
      ["test:unit"],
      isCi ? { LITRADAR_VITEST_JUNIT: "./test-results/vitest/junit.xml" } : {},
    ),
    pnpmStep(
      "Vitest Chromium components",
      ["test:browser-components"],
      isCi
        ? { LITRADAR_VITEST_JUNIT: "./test-results/vitest-browser/junit.xml" }
        : {},
    ),
    pnpmStep("Fixture-backed Playwright smoke", ["test:e2e:fixtures"]),
    ...e2eSmokeSteps(),
  ];
}

/**
 * Return separate threshold-free Rust and frontend coverage steps.
 *
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function diagnosticsSteps() {
  return [
    step("Rust coverage execution", "cargo", [
      "llvm-cov",
      "--workspace",
      "--all-features",
      "--locked",
      "--no-report",
    ]),
    step("Rust coverage HTML", "cargo", [
      "llvm-cov",
      "report",
      "--html",
      "--output-dir",
      "target/llvm-cov",
    ]),
    step("Rust coverage LCOV", "cargo", [
      "llvm-cov",
      "report",
      "--lcov",
      "--output-path",
      "target/llvm-cov/lcov.info",
    ]),
    pnpmStep("Frontend coverage", ["test:coverage"]),
  ];
}

/**
 * Select the exact steps for one public mode.
 *
 * @param {string} mode - Public mode.
 * @param {boolean} isCi - Whether CI behavior is enabled.
 * @returns {ReturnType<typeof step>[]} Ordered steps.
 */
function stepsForMode(mode, isCi) {
  if (mode === "fast") {
    return fastSteps(isCi);
  }
  if (mode === "integration") {
    return integrationSteps(isCi);
  }
  if (mode === "e2e-smoke") {
    return e2eSmokeSteps();
  }
  if (mode === "all") {
    return allSteps(isCi);
  }
  return diagnosticsSteps();
}

/**
 * Parse one mode and the optional CI switch.
 *
 * @param {string[]} args - CLI arguments.
 * @returns {{mode: string, isCi: boolean}} Parsed options.
 */
function parseArguments(args) {
  const modeArguments = args.filter((argument) => argument !== "--ci");
  const mode = modeArguments[0];
  const ciCount = args.filter((argument) => argument === "--ci").length;
  if (
    modeArguments.length !== 1 ||
    !mode ||
    !VALID_MODES.has(mode) ||
    ciCount > 1
  ) {
    throw new Error(
      "Usage: node scripts/test.mjs <fast|integration|e2e-smoke|all|diagnostics> [--ci]",
    );
  }
  return { mode, isCi: ciCount === 1 };
}

/**
 * Convert a step into a Windows-safe spawn command when pnpm resolves through a cmd shim.
 *
 * @param {ReturnType<typeof step>} definition - Step definition.
 * @returns {{command: string, args: string[]}} Spawn command.
 */
function spawnInvocation(definition) {
  if (process.platform === "win32" && definition.command === "pnpm") {
    return {
      command: process.env.ComSpec ?? "cmd.exe",
      args: ["/d", "/s", "/c", definition.command, ...definition.args],
    };
  }
  return { command: definition.command, args: definition.args };
}

/**
 * Wait for one child process to finish or fail to spawn.
 *
 * @param {import('node:child_process').ChildProcess} child - Spawned child.
 * @returns {Promise<{code: number | null, signal: NodeJS.Signals | null}>} Exit details.
 */
function waitForExit(child) {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
}

/**
 * Forward an interrupt to the active child process tree.
 *
 * @param {NodeJS.Signals} signal - Received signal.
 * @returns {void}
 */
function forwardSignal(signal) {
  if (
    !activeChild ||
    activeChild.exitCode !== null ||
    activeChild.signalCode !== null
  ) {
    return;
  }
  if (process.platform === "win32" && activeChild.pid) {
    const killer = spawn(
      "taskkill",
      ["/pid", String(activeChild.pid), "/t", "/f"],
      {
        stdio: "ignore",
        shell: false,
      },
    );
    killer.once("error", () => undefined);
    return;
  }
  activeChild.kill(signal);
}

/**
 * Run one step and record its status and duration.
 *
 * @param {ReturnType<typeof step>} definition - Step definition.
 * @param {NodeJS.ProcessEnv} sharedEnv - Shared child environment.
 * @returns {Promise<{label: string, status: string, durationMs: number}>} Step result.
 */
async function runStep(definition, sharedEnv) {
  const startedAt = performance.now();
  const invocation = spawnInvocation(definition);
  process.stdout.write(
    `\n[test:${definition.label}] ${definition.command} ${definition.args.join(" ")}\n`,
  );
  activeChild = spawn(invocation.command, invocation.args, {
    cwd: definition.cwd,
    env: { ...process.env, ...sharedEnv, ...definition.env },
    shell: false,
    stdio: "inherit",
  });
  let result;
  try {
    result = await waitForExit(activeChild);
  } catch (error) {
    const durationMs = Math.round(performance.now() - startedAt);
    activeChild = undefined;
    throw Object.assign(
      new Error(`${definition.label} could not start: ${error.message}`),
      {
        stepResult: { label: definition.label, status: "failed", durationMs },
      },
    );
  }
  activeChild = undefined;
  const durationMs = Math.round(performance.now() - startedAt);
  if (receivedSignal || result.signal) {
    throw Object.assign(new Error(`${definition.label} was interrupted`), {
      stepResult: {
        label: definition.label,
        status: "interrupted",
        durationMs,
      },
    });
  }
  if (result.code !== 0) {
    throw Object.assign(
      new Error(`${definition.label} failed with exit code ${result.code}`),
      {
        stepResult: { label: definition.label, status: "failed", durationMs },
      },
    );
  }
  return { label: definition.label, status: "passed", durationMs };
}

/**
 * Format milliseconds as a concise duration.
 *
 * @param {number} durationMs - Duration in milliseconds.
 * @returns {string} Human-readable duration.
 */
function formatDuration(durationMs) {
  return `${(durationMs / 1000).toFixed(1)}s`;
}

/**
 * Append the command result table to the GitHub Actions step summary.
 *
 * @param {string} mode - Executed public mode.
 * @param {{label: string, status: string, durationMs: number}[]} results - Completed results.
 * @returns {Promise<void>} Promise resolved after writing, or immediately outside Actions.
 */
async function appendGithubSummary(mode, results) {
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  if (!summaryPath) {
    return;
  }
  const rows = results
    .map(
      (result) =>
        `| ${result.label} | ${result.status === "passed" ? "Passed" : "Failed"} | ${formatDuration(result.durationMs)} |`,
    )
    .join("\n");
  await fs.appendFile(
    summaryPath,
    `\n## Test mode: ${mode}\n\n| Layer | Status | Duration |\n| --- | --- | ---: |\n${rows}\n`,
    "utf8",
  );
}

/**
 * Create parent directories used by deterministic CI and coverage reports.
 *
 * @param {string} mode - Executed public mode.
 * @param {boolean} isCi - Whether CI reports are enabled.
 * @returns {Promise<void>} Promise resolved after directory creation.
 */
async function prepareReportDirectories(mode, isCi) {
  const directories = [];
  if (isCi) {
    directories.push(
      path.join(WORKSPACE_ROOT, "target", "nextest", "ci"),
      path.join(APP_ROOT, "test-results", "vitest"),
      path.join(APP_ROOT, "test-results", "vitest-browser"),
      path.join(APP_ROOT, "test-results", "playwright-fixtures"),
      path.join(APP_ROOT, "test-results", "playwright-full-stack"),
      path.join(APP_ROOT, "playwright-report", "fixtures"),
      path.join(APP_ROOT, "playwright-report", "full-stack"),
    );
  }
  if (mode === "diagnostics") {
    directories.push(
      path.join(WORKSPACE_ROOT, "target", "llvm-cov"),
      path.join(APP_ROOT, "coverage"),
    );
  }
  await Promise.all(
    directories.map((directory) => fs.mkdir(directory, { recursive: true })),
  );
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    receivedSignal = signal;
    forwardSignal(signal);
  });
}

const results = [];
let executionError;
let selectedMode = "unknown";

try {
  const { mode, isCi } = parseArguments(process.argv.slice(2));
  selectedMode = mode;
  await prepareReportDirectories(mode, isCi);
  const sharedEnv = isCi ? { CI: "true", LITRADAR_TEST_CI: "true" } : {};
  for (const definition of stepsForMode(mode, isCi)) {
    try {
      results.push(await runStep(definition, sharedEnv));
    } catch (error) {
      if (error.stepResult) {
        results.push(error.stepResult);
      }
      throw error;
    }
  }
} catch (error) {
  executionError = error;
  process.stderr.write(
    `[test] ${error instanceof Error ? error.message : String(error)}\n`,
  );
} finally {
  await appendGithubSummary(selectedMode, results).catch((error) => {
    process.stderr.write(
      `[test] failed to write GitHub summary: ${error.message}\n`,
    );
    executionError ??= error;
  });
}

if (receivedSignal) {
  process.exitCode = WINDOWS_SIGNAL_EXIT_CODES[receivedSignal] ?? 1;
} else if (executionError) {
  process.exitCode = 1;
}
