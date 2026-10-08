/** Measure the actual production image using repeated isolated synthetic workloads. */
import { pathToFileURL } from "node:url";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import { randomUUID } from "node:crypto";

/** Execute three bounded native amd64 measurements and retain every raw smoke report. */
export async function profileImage() {
  const interruption = new AbortController();
  /** Preserve signal identity while allowing owned resources to be cleaned up. */
  function interrupt(signal) {
    interruption.abort(signal);
  }
  const handlers = new Map(
    ["SIGINT", "SIGTERM"].map((signal) => [signal, () => interrupt(signal)]),
  );
  const directory = "output/profiling";
  await fs.mkdir(directory, { recursive: true });
  const report = {
    status: "In Progress",
    image: "litradar:go-test-amd64",
    scope:
      "Linux production container; not comparable to historical Windows debug timings",
    rounds: [],
  };
  for (const [signal, handler] of handlers) process.on(signal, handler);
  try {
    const host = spawnSync(
      "docker",
      ["info", "--format", "{{.Architecture}}"],
      { encoding: "utf8", timeout: 30000 },
    );
    assert.equal(host.status, 0);
    assert(
      ["x86_64", "amd64"].includes(host.stdout.trim()),
      "Native amd64 host required for performance measurements",
    );
    for (let round = 1; round <= 3; round += 1) {
      interruption.signal.throwIfAborted();
      const runId = randomUUID();
      await profileRound(report, directory, round, runId, interruption.signal);
    }
    interruption.signal.throwIfAborted();
    report.status = "Passed";
    return report;
  } catch (error) {
    const failure = error instanceof Error ? error : new Error(String(error));
    if (interruption.signal.aborted)
      failure.exitCode = interruption.signal.reason === "SIGINT" ? 130 : 143;
    report.status = "Failed";
    report.error = failure.message;
    throw failure;
  } finally {
    for (const [signal, handler] of handlers)
      process.removeListener(signal, handler);
    if (interruption.signal.aborted)
      report.interruptedBy = interruption.signal.reason;
    await fs.writeFile(
      `${directory}/result.json`,
      JSON.stringify(report, null, 2) + "\n",
    );
  }
}

/** Execute one measurement, clean its ownership scope and retain failure precedence.
 *
 * @param {object} report - Accumulated measurement report.
 * @param {string} directory - Raw log destination.
 * @param {number} round - Current measurement number.
 * @param {string} runId - Unpredictable resource ownership identity.
 * @param {AbortSignal} signal - Original interruption signal.
 * @returns {Promise<void>} Completion of one validated observation.
 */
async function profileRound(report, directory, round, runId, signal) {
  const result = await runSmoke(report.image, runId, signal);
  let cleanupErrors;
  try {
    await fs.writeFile(
      `${directory}/round-${round}.log`,
      result.stdout + "\n" + result.stderr,
    );
  } finally {
    cleanupErrors = cleanupRun(runId);
  }
  report.lastExecution = {
    round,
    runId,
    exitCode: result.status,
    error: result.error?.message,
    cleanupErrors,
  };
  assert.equal(
    cleanupErrors.length,
    0,
    `Profile cleanup failed: ${cleanupErrors.join("; ")}; child: ${result.error?.message ?? result.status}`,
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, `Profile round ${round} failed`);
  const observation = JSON.parse(
    await fs.readFile("test-results/container-smoke/summary.json", "utf8"),
  );
  appendProfileObservation(report, observation);
}

/** Validate and append the original smoke observation after successful cleanup.
 *
 * @param {object} report - Accumulated measurement report.
 * @param {object} observation - Parsed smoke report.
 * @returns {void} Completion of the observation invariants.
 */
function appendProfileObservation(report, observation) {
  assert.equal(observation.architecture, "amd64");
  assert.equal(observation.status, "passed");
  assert.equal(
    observation.profile.requests.reduce(
      (sum, entry) => sum + entry.completed,
      0,
    ),
    200,
  );
  if (report.rounds.length)
    assert.equal(observation.imageId, report.rounds[0].imageId);
  report.rounds.push(observation);
}

/** Remove only resources labeled with this measurement's unpredictable ownership ID. */
function cleanupRun(runId) {
  const errors = [];
  for (const kind of ["container", "volume"]) {
    cleanupProfileKind(kind, runId, errors);
  }
  return errors;
}

/** Inventory and remove one resource kind without abandoning later kinds.
 *
 * @param {string} kind - Original Docker resource kind.
 * @param {string} runId - Measurement ownership identity.
 * @param {string[]} errors - Caller-owned ordered cleanup errors.
 * @returns {void} Completion of this resource inventory.
 */
function cleanupProfileKind(kind, runId, errors) {
  const inventory = spawnSync(
    "docker",
    [
      kind,
      "ls",
      ...(kind === "container" ? ["--all"] : []),
      "--quiet",
      "--filter",
      `label=org.litradar.smoke-run=${runId}`,
    ],
    { encoding: "utf8", timeout: 30000, windowsHide: true },
  );
  if (inventory.error || inventory.status !== 0) {
    errors.push(
      `Unable to inventory ${kind}: ${inventory.error?.message ?? inventory.stderr}`,
    );
    return;
  }
  for (const identity of inventory.stdout
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)) {
    removeProfileResource(kind, identity, errors);
  }
}

/** Retain a failed owned-resource removal while allowing remaining cleanup.
 *
 * @param {string} kind - Docker resource kind.
 * @param {string} identity - Exact identity returned by owned inventory.
 * @param {string[]} errors - Caller-owned ordered cleanup errors.
 * @returns {void} Completion of this removal attempt.
 */
function removeProfileResource(kind, identity, errors) {
  const removal = spawnSync("docker", [kind, "rm", "--force", identity], {
    encoding: "utf8",
    timeout: 30000,
    windowsHide: true,
  });
  if (removal.error || removal.status !== 0)
    errors.push(
      `Unable to remove ${kind} ${identity}: ${removal.error?.message ?? removal.stderr}`,
    );
}

/** Force the owned smoke process tree to exit at its deadline before resource cleanup. */
function runSmoke(image, runId, signal) {
  return new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      ["tests/container-smoke.mjs", image, "--profile"],
      {
        env: { ...process.env, LITRADAR_SMOKE_RUN_ID: runId },
        windowsHide: true,
        detached: process.platform !== "win32",
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    const result = { stdout: "", stderr: "", status: null, error: undefined };
    /** Terminate only the process tree created by this measurement. */
    function terminate(reason) {
      result.error = new Error(reason);
      if (!child.pid) return;
      if (process.platform === "win32") {
        const killed = spawnSync(
          "taskkill",
          ["/pid", String(child.pid), "/t", "/f"],
          { encoding: "utf8", windowsHide: true, timeout: 30000 },
        );
        if (killed.error || killed.status !== 0) {
          result.stderr += killed.stderr ?? "";
          child.kill("SIGKILL");
        }
      } else {
        try {
          process.kill(-child.pid, "SIGKILL");
        } catch (error) {
          if (error.code !== "ESRCH") result.stderr += error.message;
        }
      }
    }
    const timer = setTimeout(
      () => terminate("Profile exceeded 300 seconds"),
      300000,
    );
    const interrupt = () =>
      terminate("Profile interrupted by " + signal.reason);
    signal.addEventListener("abort", interrupt, { once: true });
    if (signal.aborted) interrupt();
    for (const stream of ["stdout", "stderr"])
      child[stream].on("data", (chunk) => {
        result[stream] += chunk.toString();
        if (result[stream].length > 8 * 1024 * 1024)
          terminate("Profile output exceeded bound");
      });
    child.once("error", (error) => {
      result.error = error;
    });
    child.once("close", (status) => {
      clearTimeout(timer);
      signal.removeEventListener("abort", interrupt);
      result.status = status;
      resolve(result);
    });
  });
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    console.log(await profileImage());
  } catch (error) {
    console.error(error);
    process.exitCode = error.exitCode ?? 1;
  }
}
