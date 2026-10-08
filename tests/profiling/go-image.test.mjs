/** Verify profile interruption propagates and cleans only the owned run resources. */
import assert from "node:assert/strict";
import childProcess from "node:child_process";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import promises from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { profileImage } from "./go-image.mjs";

test("SIGTERM stops the active measurement and removes its containers and volumes", async () => {
  const original = {
    spawn: childProcess.spawn,
    spawnSync: childProcess.spawnSync,
    kill: process.kill,
    cwd: process.cwd(),
  };
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "profile-interrupt-"),
  );
  const calls = [];
  let child;
  let fallback;
  const handlerCount = process.listenerCount("SIGTERM");
  try {
    process.chdir(directory);
    childProcess.spawn = () => {
      child = new EventEmitter();
      child.pid = 1234567;
      child.stdout = new EventEmitter();
      child.stderr = new EventEmitter();
      setImmediate(() => {
        process.emit("SIGTERM");
        fallback = setTimeout(() => child.emit("close", 0), 30);
      });
      return child;
    };
    childProcess.spawnSync = (command, args) => {
      calls.push([command, ...args]);
      if (command === "taskkill")
        queueMicrotask(() => child.emit("close", null));
      const stdout =
        args[0] === "info"
          ? "amd64"
          : args[1] === "ls"
            ? "owned-" + args[0]
            : "";
      return { status: 0, stdout, stderr: "" };
    };
    process.kill = (pid, signal) => {
      calls.push(["kill", pid, signal]);
      queueMicrotask(() => child.emit("close", null));
    };
    syncBuiltinESMExports();
    await assert.rejects(
      profileImage(),
      (error) => error.exitCode === 143 && error.message.includes("SIGTERM"),
    );
    assert(
      calls.some(
        (entry) =>
          entry[0] === (process.platform === "win32" ? "taskkill" : "kill"),
      ),
    );
    for (const kind of ["container", "volume"]) {
      assert(
        calls.some(
          (entry) =>
            entry[1] === kind &&
            entry[2] === "ls" &&
            entry.some((value) =>
              String(value).startsWith("label=org.litradar.smoke-run="),
            ),
        ),
      );
      assert(
        calls.some(
          (entry) =>
            entry[1] === kind &&
            entry[2] === "rm" &&
            entry[4] === "owned-" + kind,
        ),
      );
    }
    const report = JSON.parse(
      fs.readFileSync("output/profiling/result.json", "utf8"),
    );
    assert.equal(report.status, "Failed");
    assert.equal(report.interruptedBy, "SIGTERM");
    assert.equal(process.listenerCount("SIGTERM"), handlerCount);
  } finally {
    clearTimeout(fallback);
    childProcess.spawn = original.spawn;
    childProcess.spawnSync = original.spawnSync;
    process.kill = original.kill;
    syncBuiltinESMExports();
    process.chdir(original.cwd);
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

/** Return only the resources belonging to the active synthetic measurement. */
function profileDockerResult(args, state, options) {
  if (args[0] === "info") return { status: 0, stdout: "amd64", stderr: "" };
  const kind = args[0];
  assert(["container", "volume"].includes(kind));
  if (args[1] === "ls") {
    state.events.push(["inventory", kind, state.round]);
    if (kind === options.inventoryFailure)
      return { status: 1, stdout: "", stderr: "inventory failed" };
    return {
      status: 0,
      stdout: `${kind}-${state.runId}\n${kind}-${state.runId}-second\n`,
      stderr: "",
    };
  }
  assert.equal(args[1], "rm");
  state.events.push(["remove", kind, args[3]]);
  return {
    status: kind === options.removalFailure ? 1 : 0,
    stdout: "",
    stderr: "removal failed",
  };
}

/** Complete a mocked child only after its output or interruption is observed. */
function createProfileChild(args, configuration, state, options) {
  assert.deepEqual(args, [
    "tests/container-smoke.mjs",
    "litradar:go-test-amd64",
    "--profile",
  ]);
  state.round += 1;
  state.runId = configuration.env.LITRADAR_SMOKE_RUN_ID;
  assert.match(state.runId, /^[0-9a-f-]{36}$/);
  state.runIds.push(state.runId);
  state.events.push(["spawn", state.round]);
  const child = new EventEmitter();
  child.pid = 1234567;
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.on("close", () => state.events.push(["close", state.round]));
  state.child = child;
  setImmediate(() => {
    child.stdout.emit("data", Buffer.from("synthetic stdout"));
    child.stderr.emit("data", Buffer.from("synthetic stderr"));
    if (options.signal) {
      process.emit(options.signal);
      return;
    }
    if (options.childError) child.emit("error", new Error("child failed"));
    child.emit("close", options.childStatus ?? 0);
  });
  return child;
}

/** Provide an isolated valid report, optionally challenging a round invariant. */
function profileObservation(state, options) {
  if (options.abortAfterRound === state.round) process.emit("SIGTERM");
  return JSON.stringify({
    architecture: options.architecture ?? "amd64",
    status: options.observationStatus ?? "passed",
    imageId: options.imageMismatch && state.round === 2 ? "second" : "first",
    profile: { requests: [{ completed: options.completed ?? 200 }] },
  });
}

/** Run profile orchestration with all Docker and process boundaries mocked. */
async function withProfileScenario(options, verify) {
  const original = {
    spawn: childProcess.spawn,
    spawnSync: childProcess.spawnSync,
    kill: process.kill,
    readFile: promises.readFile,
    writeFile: promises.writeFile,
    cwd: process.cwd(),
  };
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "profile-contract-"));
  const state = { round: 0, runIds: [], commands: [], events: [] };
  const listeners = ["SIGINT", "SIGTERM"].map((signal) =>
    process.listenerCount(signal),
  );
  try {
    process.chdir(directory);
    childProcess.spawn = (executable, args, configuration) => {
      assert.equal(executable, process.execPath);
      return createProfileChild(args, configuration, state, options);
    };
    childProcess.spawnSync = (command, args, configuration) => {
      state.commands.push({ command, args, configuration, round: state.round });
      if (command === "taskkill") {
        queueMicrotask(() => state.child.emit("close", null));
        return { status: 0, stdout: "", stderr: "" };
      }
      assert.equal(command, "docker");
      return profileDockerResult(args, state, options);
    };
    process.kill = (pid, signal) => {
      state.termination = [pid, signal];
      queueMicrotask(() => state.child.emit("close", null));
    };
    promises.readFile = async () => profileObservation(state, options);
    promises.writeFile = async (filename, data) => {
      if (options.logWriteFailure && filename.endsWith(".log"))
        throw new Error("log write failed");
      return original.writeFile(filename, data);
    };
    syncBuiltinESMExports();
    try {
      state.result = await profileImage();
    } catch (error) {
      state.error = error;
    }
    state.report = JSON.parse(
      fs.readFileSync("output/profiling/result.json", "utf8"),
    );
    assert.deepEqual(
      ["SIGINT", "SIGTERM"].map((signal) => process.listenerCount(signal)),
      listeners,
    );
    await verify(state);
  } finally {
    childProcess.spawn = original.spawn;
    childProcess.spawnSync = original.spawnSync;
    process.kill = original.kill;
    promises.readFile = original.readFile;
    promises.writeFile = original.writeFile;
    syncBuiltinESMExports();
    process.chdir(original.cwd);
    assert.equal(
      path.dirname(path.resolve(directory)),
      path.resolve(os.tmpdir()),
    );
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

/** Verify exact run ownership and close-before-cleanup order for every round. */
function assertProfileOwnership(state) {
  for (const [index, runId] of state.runIds.entries()) {
    const round = index + 1;
    for (const kind of ["container", "volume"]) {
      const inventory = state.commands.find(
        (entry) =>
          entry.round === round &&
          entry.args[0] === kind &&
          entry.args[1] === "ls",
      );
      assert.deepEqual(inventory.args, [
        kind,
        "ls",
        ...(kind === "container" ? ["--all"] : []),
        "--quiet",
        "--filter",
        `label=org.litradar.smoke-run=${runId}`,
      ]);
      assert.equal(inventory.configuration.timeout, 30000);
      assert.equal(inventory.configuration.windowsHide, true);
    }
    const closeIndex = state.events.findIndex(
      (entry) => entry[0] === "close" && entry[1] === round,
    );
    const inventoryIndex = state.events.findIndex(
      (entry) => entry[0] === "inventory" && entry[2] === round,
    );
    assert(closeIndex >= 0 && inventoryIndex > closeIndex);
  }
}

/** Verify removal is limited to the exact identities returned by owned inventory. */
function assertProfileRemovals(state, kinds = ["container", "volume"]) {
  const actual = state.commands.filter((entry) => entry.args[1] === "rm");
  const expected = state.runIds.flatMap((runId) =>
    kinds.flatMap((kind) => [
      [kind, "rm", "--force", `${kind}-${runId}`],
      [kind, "rm", "--force", `${kind}-${runId}-second`],
    ]),
  );
  assert.deepEqual(
    actual.map((entry) => entry.args),
    expected,
  );
}

test("three profile rounds preserve image identity and exact owned cleanup", async () => {
  await withProfileScenario({}, async (state) => {
    assert.ifError(state.error);
    assert.equal(state.report.status, "Passed");
    assert.equal(state.report.rounds.length, 3);
    assert.equal(new Set(state.runIds).size, 3);
    assert.equal(state.report.lastExecution.round, 3);
    assertProfileOwnership(state);
    assertProfileRemovals(state);
  });
});

test("SIGINT retains exit130 and terminates only the owned process tree", async () => {
  await withProfileScenario({ signal: "SIGINT" }, async (state) => {
    assert.equal(state.error.exitCode, 130);
    assert.equal(state.report.interruptedBy, "SIGINT");
    assert.match(state.report.error, /Profile interrupted by SIGINT/);
    assert.equal(state.runIds.length, 1);
    if (process.platform === "win32") {
      const termination = state.commands.find(
        (entry) => entry.command === "taskkill",
      );
      assert.deepEqual(termination.args, ["/pid", "1234567", "/t", "/f"]);
    } else {
      assert.deepEqual(state.termination, [-1234567, "SIGKILL"]);
    }
    assertProfileOwnership(state);
    assertProfileRemovals(state);
  });
});

test("container inventory failure still cleans volumes and reports the cleanup error", async () => {
  await withProfileScenario(
    { inventoryFailure: "container", childError: true },
    async (state) => {
      assert.match(
        state.error.message,
        /Profile cleanup failed: Unable to inventory container/,
      );
      assert.equal(state.report.lastExecution.error, "child failed");
      assert.deepEqual(state.report.lastExecution.cleanupErrors, [
        "Unable to inventory container: inventory failed",
      ]);
      assertProfileOwnership(state);
      assertProfileRemovals(state, ["volume"]);
    },
  );
});

test("SIGTERM cleanup failures outrank child interruption without losing exit143", async () => {
  await withProfileScenario(
    { signal: "SIGTERM", removalFailure: "container" },
    async (state) => {
      assert.equal(state.error.exitCode, 143);
      assert.match(state.error.message, /^Profile cleanup failed:/);
      assert.equal(state.report.interruptedBy, "SIGTERM");
      assert.equal(
        state.report.lastExecution.error,
        "Profile interrupted by SIGTERM",
      );
      assert.equal(state.report.lastExecution.cleanupErrors.length, 2);
      assertProfileOwnership(state);
      assertProfileRemovals(state);
    },
  );
});

test("log write failure still cleans the run before final failure reporting", async () => {
  await withProfileScenario({ logWriteFailure: true }, async (state) => {
    assert.equal(state.error.message, "log write failed");
    assert.equal(state.report.status, "Failed");
    assert.equal(state.report.lastExecution, undefined);
    assertProfileOwnership(state);
    assertProfileRemovals(state);
  });
});

test("child exit failure is retained after successful cleanup", async () => {
  await withProfileScenario({ childStatus: 7 }, async (state) => {
    assert.match(state.error.message, /Profile round 1 failed/);
    assert.equal(state.report.lastExecution.exitCode, 7);
    assert.deepEqual(state.report.lastExecution.cleanupErrors, []);
    assertProfileRemovals(state);
  });
});

test("different round image identities are rejected after cleanup", async () => {
  await withProfileScenario({ imageMismatch: true }, async (state) => {
    assert(state.error instanceof Error);
    assert.equal(state.runIds.length, 2);
    assert.equal(state.report.rounds.length, 1);
    assert.equal(state.report.lastExecution.round, 2);
    assertProfileOwnership(state);
    assertProfileRemovals(state);
  });
});

for (const options of [
  { architecture: "arm64" },
  { observationStatus: "failed" },
  { completed: 199 },
]) {
  test(`profile observation contract rejects ${JSON.stringify(options)}`, async () => {
    await withProfileScenario(options, async (state) => {
      assert(state.error instanceof Error);
      assert.equal(state.report.status, "Failed");
      assert.equal(state.report.rounds.length, 0);
      assertProfileRemovals(state);
    });
  });
}

for (const round of [1, 3]) {
  test(`abort after observation ${round} prevents a successful final report`, async () => {
    await withProfileScenario({ abortAfterRound: round }, async (state) => {
      assert.equal(state.error.exitCode, 143);
      assert.equal(state.runIds.length, round);
      assert.equal(state.report.rounds.length, round);
      assert.equal(state.report.status, "Failed");
      assertProfileOwnership(state);
      assertProfileRemovals(state);
    });
  });
}
