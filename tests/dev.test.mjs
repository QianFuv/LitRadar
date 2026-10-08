/** Verify development lifecycle and bounded process-tree ownership through local mocks. */
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

/** Create an owned child whose exit state and promise advance together. */
function ownedChild(pid = 123) {
  const child = Object.assign(new EventEmitter(), {
    pid,
    exitCode: null,
    signalCode: null,
  });
  let resolveExit;
  const exited = new Promise((resolve) => {
    resolveExit = resolve;
  });
  /** Publish the simulated process exit before resolving the caller's exit promise. */
  function exit(code = 0, signal = null) {
    child.exitCode = code;
    child.signalCode = signal;
    child.emit("close", code, signal);
    resolveExit({ code, signal });
  }
  return { child, exited, exit };
}

/** Keep an unresolved race participant inert without allocating a real timer. */
function pendingDelay() {
  return new Promise(() => {});
}

/** Advance simulated grace or final deadlines immediately. */
function expiredDelay() {
  return Promise.resolve();
}

/** Simulate an exclusive loopback probe without creating a real listener. */
function mockPortListener(events, occupiedPort) {
  const listener = new EventEmitter();
  /** Record the exact ownership boundary and emit a controlled occupancy error. */
  listener.listen = function listen(options, ready) {
    assert.equal(options.host, "127.0.0.1");
    assert.equal(options.exclusive, true);
    events.push(["port", options.port]);
    queueMicrotask(() => {
      if (options.port === occupiedPort)
        listener.emit("error", new Error("occupied"));
      else ready();
    });
  };
  /** Close only the temporary simulated probe. */
  listener.close = function close(done) {
    events.push(["port-close"]);
    done();
  };
  return listener;
}

/** Execute real lifecycle functions with process, filesystem, network and timers mocked. */
function createDevHarness(settings = {}) {
  const events = [];
  const mockProcess = Object.assign(new EventEmitter(), {
    platform: settings.platform ?? "win32",
    argv: ["node", "fixture", ...(settings.args ?? [])],
    env: {},
    execPath: "fixture-node",
  });
  /** Capture diagnostics and optionally interrupt once both services report ready. */
  function writeOutput(value) {
    events.push(["stdout", value]);
    if (value.startsWith("[dev] Ready:")) settings.onReady?.(mockProcess);
  }
  mockProcess.stdout = { write: writeOutput };
  mockProcess.stderr = { write: (value) => events.push(["stderr", value]) };
  /** Record signals to the supplied owned group without touching a host process. */
  mockProcess.kill = function kill(pid, signal) {
    assert(pid < 0, "POSIX shutdown must target the owned group");
    events.push(["kill", pid, signal]);
    settings.kill?.(pid, signal);
  };
  /** Admit only mocked process starts and exact Windows tree-termination options. */
  function spawnMock(command, args, options) {
    events.push(["spawn", command, args, options]);
    if (command !== "taskkill") {
      assert(settings.spawn, "Unexpected service process start");
      return settings.spawn(command, args, options);
    }
    assert.deepEqual(args, ["/pid", args[1], "/t", "/f"]);
    assert.match(args[1], /^\d+$/);
    assert.deepEqual(options, {
      stdio: "ignore",
      shell: false,
      windowsHide: true,
    });
    const killer = new EventEmitter();
    queueMicrotask(() => {
      const failure = settings.killerError?.(args);
      if (failure) killer.emit("error", failure);
      else {
        settings.onTaskkill?.(args);
        killer.emit("exit", settings.killerCode ?? 0);
      }
    });
    return killer;
  }
  /** Observe prerequisite access while keeping installation and host files untouched. */
  async function accessMock(filename) {
    events.push(["access", filename]);
    if (settings.accessError) throw settings.accessError;
  }
  /** Return only the explicitly configured deployment-key metadata. */
  async function statMock(filename) {
    events.push(["stat", filename]);
    return {
      size: settings.keySize ?? 32,
      isFile: () => settings.isKeyFile !== false,
    };
  }
  /** Record build-directory intent without creating a filesystem path. */
  async function mkdirMock(directory, options) {
    events.push(["mkdir", directory, options]);
  }
  /** Record exact timeout/ref policy and use the test's controlled race participant. */
  function delayMock(milliseconds, value, options) {
    events.push(["delay", milliseconds, value, options]);
    return (settings.delay ?? pendingDelay)(milliseconds, value, options);
  }
  /** Consume readiness response bodies before admitting either service. */
  async function fetchMock(url, options) {
    assert(options.signal instanceof AbortSignal);
    events.push(["ready", url]);
    return {
      ok: true,
      body: {
        cancel: async () => {
          events.push(["cancel", url]);
        },
      },
    };
  }
  const moduleUrl = new URL("../scripts/dev.mjs", import.meta.url);
  const source = fs
    .readFileSync(moduleUrl, "utf8")
    .replaceAll("\r\n", "\n")
    .replace(/^import[\s\S]*?;$/gm, "")
    .replaceAll("import.meta.url", JSON.stringify(moduleUrl.href));
  const entry = source.lastIndexOf("\ntry {\n  await main();");
  assert(
    entry > 0,
    "CLI entry must be isolated before executing mocked functions",
  );
  const load = new Function(
    "buildSimple",
    "simpleBuildEnvironment",
    "spawn",
    "access",
    "mkdir",
    "stat",
    "createServer",
    "path",
    "delay",
    "fileURLToPath",
    "process",
    "fetch",
    source.slice(0, entry) +
      "\nasync function runEntry() {" +
      source.slice(entry) +
      "}\nreturn {main, stopChild, startChild, buildBackend, runEntry, children: CHILDREN};",
  );
  const functions = load(
    async () => {
      events.push(["native-build"]);
    },
    () => ({
      ...mockProcess.env,
      CGO_CFLAGS: "-DLITRADAR_SIMPLE_INPUT_test=1",
    }),
    spawnMock,
    accessMock,
    mkdirMock,
    statMock,
    () => mockPortListener(events, settings.occupiedPort),
    path,
    delayMock,
    fileURLToPath,
    mockProcess,
    fetchMock,
  );
  return { ...functions, events, process: mockProcess };
}

/** Select observable process actions without consulting production state decisions. */
function processActions(harness) {
  return harness.events.filter((event) =>
    ["spawn", "kill", "delay"].includes(event[0]),
  );
}

/** Preserve missing-PID and already-exited Windows early returns. */
async function skipsInactiveWindowsChildren() {
  for (const pid of [undefined, 0]) {
    const harness = createDevHarness();
    await harness.stopChild({ child: { pid }, exited: pendingDelay() });
    assert.deepEqual(processActions(harness), []);
  }
  const entry = ownedChild();
  entry.exit();
  const harness = createDevHarness();
  await harness.stopChild(entry);
  assert.deepEqual(processActions(harness), []);
}

/** Let SIGINT graceful exit complete before considering forced Windows termination. */
async function honorsWindowsInterruptGrace() {
  const entry = ownedChild();
  const harness = createDevHarness();
  harness.process.emit("SIGINT");
  queueMicrotask(() => entry.exit());
  await harness.stopChild(entry);
  assert.deepEqual(processActions(harness), [
    ["delay", 10000, undefined, { ref: false }],
  ]);
}

/** Force only the owned Windows tree after grace expiry, with SIGTERM skipping grace. */
async function forcesWindowsOwnedTree() {
  for (const signal of ["SIGINT", "SIGTERM"]) {
    const entry = ownedChild();
    const harness = createDevHarness({
      delay: expiredDelay,
      onTaskkill: () => entry.exit(),
    });
    harness.process.emit(signal);
    await harness.stopChild(entry);
    const actions = processActions(harness);
    const spawn = actions.find((event) => event[0] === "spawn");
    assert.deepEqual(spawn, [
      "spawn",
      "taskkill",
      ["/pid", "123", "/t", "/f"],
      { stdio: "ignore", shell: false, windowsHide: true },
    ]);
    assert.equal(actions[0][0], signal === "SIGINT" ? "delay" : "spawn");
    assert.equal(
      actions.filter((event) => event[0] === "delay").length,
      signal === "SIGINT" ? 2 : 1,
    );
    assert.equal(entry.child.exitCode, 0);
  }
}

/** Preserve taskkill startup, nonzero-exit and final-deadline failures. */
async function rejectsWindowsTerminationFailures() {
  const startup = createDevHarness({
    killerError: () => new Error("killer startup"),
  });
  await assert.rejects(startup.stopChild(ownedChild()), /killer startup/);
  assert.equal(processActions(startup).length, 1);
  const failed = createDevHarness({ killerCode: 2 });
  await assert.rejects(
    failed.stopChild(ownedChild()),
    /Could not stop process tree 123/,
  );
  assert.equal(processActions(failed).length, 1);
  const timeout = createDevHarness({ delay: expiredDelay });
  await assert.rejects(
    timeout.stopChild(ownedChild()),
    /Process 123 did not exit after shutdown/,
  );
}

/** Accept a nonzero killer exit if the owned child meanwhile completed. */
async function acceptsConcurrentWindowsExit() {
  const entry = ownedChild();
  const harness = createDevHarness({
    killerCode: 2,
    onTaskkill: () => entry.exit(),
  });
  await harness.stopChild(entry);
  assert.equal(entry.child.exitCode, 0);
  assert.deepEqual(
    processActions(harness).map((event) => event[0]),
    ["spawn", "delay"],
  );
}

/** Retain both POSIX signals even when the exit promise resolves during the first wait. */
async function preservesPosixSignalSequence() {
  const entry = ownedChild();
  const harness = createDevHarness({
    platform: "linux",
    kill: () => entry.exit(),
  });
  await harness.stopChild(entry);
  assert.deepEqual(processActions(harness), [
    ["kill", -123, "SIGTERM"],
    ["delay", 10000, undefined, { ref: false }],
    ["kill", -123, "SIGKILL"],
    ["delay", 10000, undefined, { ref: false }],
  ]);
}

/** Ignore only ESRCH while preserving other signal failures and the final deadline. */
async function preservesPosixFailures() {
  const entry = ownedChild();
  entry.exit();
  const missing = createDevHarness({
    platform: "linux",
    kill: () => {
      throw Object.assign(new Error("gone"), { code: "ESRCH" });
    },
  });
  await missing.stopChild(entry);
  assert.equal(
    processActions(missing).filter((event) => event[0] === "kill").length,
    2,
  );
  const denied = createDevHarness({
    platform: "linux",
    kill: () => {
      throw Object.assign(new Error("denied"), { code: "EPERM" });
    },
  });
  await assert.rejects(denied.stopChild(ownedChild()), /denied/);
  assert.equal(processActions(denied).length, 1);
  const killDenied = createDevHarness({
    platform: "linux",
    delay: expiredDelay,
    kill: (pid, signal) => {
      if (signal === "SIGKILL") {
        throw Object.assign(new Error("kill denied"), { code: "EPERM" });
      }
    },
  });
  await assert.rejects(killDenied.stopChild(ownedChild()), /kill denied/);
  assert.deepEqual(processActions(killDenied), [
    ["kill", -123, "SIGTERM"],
    ["delay", 10000, undefined, { ref: false }],
    ["kill", -123, "SIGKILL"],
  ]);
  const timeout = createDevHarness({ platform: "linux", delay: expiredDelay });
  await assert.rejects(
    timeout.stopChild(ownedChild()),
    /did not exit after shutdown/,
  );
  assert.deepEqual(
    processActions(timeout).filter((event) => event[0] === "kill"),
    [
      ["kill", -123, "SIGTERM"],
      ["kill", -123, "SIGKILL"],
    ],
  );
}

/** Keep help and invalid arguments ahead of any filesystem or process action. */
async function admitsArgumentsBeforePreflight() {
  const help = createDevHarness({ args: ["--help"] });
  await help.runEntry();
  assert.match(help.events[0][1], /Usage: node scripts\/dev.mjs/);
  assert(
    !help.events.some((event) =>
      ["access", "stat", "spawn"].includes(event[0]),
    ),
  );
  assert.equal(help.process.exitCode, undefined);
  const invalid = createDevHarness({ args: ["--unexpected"] });
  await invalid.runEntry();
  assert.equal(invalid.process.exitCode, 1);
  assert.deepEqual(
    invalid.events.map((event) => event[0]),
    ["stderr"],
  );
}

/** Register an isolated POSIX group before observing its real startup listener. */
async function startsDetachedPosixGroup() {
  const entry = ownedChild();
  const harness = createDevHarness({
    platform: "linux",
    spawn: () => entry.child,
  });
  const started = harness.startChild(
    "fixture",
    "fixture-command",
    ["argument"],
    "fixture-root",
  );
  assert.deepEqual(processActions(harness), [
    [
      "spawn",
      "fixture-command",
      ["argument"],
      {
        cwd: "fixture-root",
        stdio: "inherit",
        env: { CGO_ENABLED: "1" },
        windowsHide: true,
        shell: false,
        detached: true,
      },
    ],
  ]);
  assert.equal(harness.children[0], started);
  entry.exit(7);
  assert.deepEqual(await started.exited, {
    label: "fixture",
    code: 7,
    signal: null,
  });
}

/** Reject installation, key and port failures without starting or terminating a host process. */
async function rejectsUnsafePreflight() {
  const missing = createDevHarness({ accessError: new Error("missing") });
  await missing.runEntry();
  assert.equal(missing.process.exitCode, 1);
  assert.deepEqual(
    missing.events.map((event) => event[0]),
    ["access", "stderr"],
  );
  for (const settings of [
    { keySize: 31 },
    { isKeyFile: false },
    { occupiedPort: 8000 },
    { occupiedPort: 8001 },
  ]) {
    const harness = createDevHarness(settings);
    await harness.runEntry();
    assert.equal(harness.process.exitCode, 1);
    assert.deepEqual(processActions(harness), []);
    assert(harness.events.some((event) => event[0] === "stderr"));
  }
}

/** Create controlled build and service children, observing startup through the real owners. */
function developmentChildren(settings = {}) {
  const build = ownedChild(111),
    backend = ownedChild(222),
    frontend = ownedChild(333);
  let harness;
  /** Supply only the expected Go build, backend and frontend process roles. */
  function spawn(command, args) {
    if (command === "go") {
      queueMicrotask(() => {
        if (settings.interruptBuild) harness.process.emit("SIGINT");
        else build.exit();
      });
      return build.child;
    }
    if (command === "fixture-node") return frontend.child;
    if (settings.backendFails) queueMicrotask(() => backend.exit(9));
    return backend.child;
  }
  /** Complete only the exact owned PID named by the simulated Windows tree command. */
  function onTaskkill(args) {
    const entry = [build, backend, frontend].find(
      (value) => String(value.child.pid) === args[1],
    );
    assert(
      entry,
      "Termination must stay within the registered fixture children",
    );
    entry.exit();
  }
  harness = createDevHarness({
    args: ["--project-root", "fixture-project-root"],
    delay: expiredDelay,
    spawn,
    onTaskkill,
    onReady: (process) => process.emit("SIGTERM"),
    ...settings,
  });
  return { harness, build, backend, frontend };
}

/** Keep an interrupted build registered until entry cleanup terminates its owned tree. */
async function cleansInterruptedBuild() {
  const { harness, build } = developmentChildren({ interruptBuild: true });
  await harness.runEntry();
  assert.equal(harness.children.length, 1);
  assert.equal(build.child.exitCode, 0);
  assert.deepEqual(
    processActions(harness)
      .filter((event) => event[0] === "spawn")
      .map((event) => event[1]),
    ["go", "taskkill"],
  );
  assert.equal(harness.process.exitCode, undefined);
}

/** Launch both exact services, consume readiness bodies and clean only their registered PIDs. */
async function supervisesOwnedServices() {
  const { harness, backend, frontend } = developmentChildren();
  await harness.runEntry();
  const starts = harness.events.filter(
    (event) => event[0] === "spawn" && event[1] !== "taskkill",
  );
  const workspace = path.resolve(
    fileURLToPath(new URL("../", import.meta.url)),
  );
  const projectRoot = path.resolve("fixture-project-root");
  const executable = path.join(workspace, "target", "go", "litradar.exe");
  assert.deepEqual(
    starts.map((event) => event[1]),
    ["go", executable, "fixture-node"],
  );
  assert.equal(starts[0][3].env.CGO_CFLAGS, "-DLITRADAR_SIMPLE_INPUT_test=1");
  assert(
    harness.events.findIndex((event) => event[0] === "native-build") <
      harness.events.indexOf(starts[0]),
  );
  assert.deepEqual(starts[0][2], [
    "build",
    "-mod=readonly",
    "-tags",
    "sqlite_fts5,sqlite_dbstat",
    "-o",
    executable,
    "./cmd/litradar",
  ]);
  assert.deepEqual(starts[1][2], [
    "serve",
    "--development",
    "--host",
    "127.0.0.1",
    "--port",
    "8001",
    "--project-root",
    projectRoot,
    "--secret-key-file",
    path.join(projectRoot, "secrets", "litradar.key"),
  ]);
  assert.deepEqual(starts[2][2], [
    path.join(workspace, "app", "node_modules", "next", "dist", "bin", "next"),
    "dev",
    "--hostname",
    "127.0.0.1",
    "--port",
    "8000",
  ]);
  for (const start of starts) {
    assert.equal(start[3].windowsHide, true);
    assert.equal(start[3].shell, false);
    assert.equal(start[3].detached, false);
    assert.equal(start[3].env.CGO_ENABLED, "1");
  }
  assert.deepEqual(
    starts.map((event) => event[3].cwd),
    [workspace, workspace, path.join(workspace, "app")],
  );
  assert.equal(
    harness.events.filter((event) => event[0] === "cancel").length,
    2,
  );
  assert.equal(harness.children.length, 2);
  assert.equal(backend.child.exitCode, 0);
  assert.equal(frontend.child.exitCode, 0);
  assert.equal(harness.process.exitCode, undefined);
}

/** Report unexpected child exit while still cleaning its surviving sibling. */
async function cleansAfterServiceFailure() {
  const { harness, frontend } = developmentChildren({ backendFails: true });
  await harness.runEntry();
  assert.equal(harness.process.exitCode, 1);
  assert(
    harness.events.some(
      (event) =>
        event[0] === "stderr" && /Go API exited unexpectedly: 9/.test(event[1]),
    ),
  );
  assert.equal(frontend.child.exitCode, 0);
  assert.deepEqual(
    harness.events
      .filter((event) => event[0] === "spawn" && event[1] === "taskkill")
      .map((event) => event[2][1]),
    ["333"],
  );
}

/** Await every registered cleanup even when the first tree termination rejects. */
async function continuesAfterCleanupFailure() {
  const { harness, frontend } = developmentChildren({
    killerError: (args) =>
      args[1] === "222" ? new Error("owned killer failure") : null,
  });
  await harness.runEntry();
  assert.equal(harness.process.exitCode, 1);
  assert.equal(frontend.child.exitCode, 0);
  assert.deepEqual(
    harness.events
      .filter((event) => event[0] === "spawn" && event[1] === "taskkill")
      .map((event) => event[2][1]),
    ["222", "333"],
  );
  assert(
    harness.events.some(
      (event) => event[0] === "stderr" && /owned killer failure/.test(event[1]),
    ),
  );
}

test(
  "Windows inactive children require no process operation",
  skipsInactiveWindowsChildren,
);
test("Windows SIGINT honors graceful completion", honorsWindowsInterruptGrace);
test(
  "Windows forced shutdown stays within the owned tree",
  forcesWindowsOwnedTree,
);
test(
  "Windows killer failures and deadlines propagate",
  rejectsWindowsTerminationFailures,
);
test(
  "Windows concurrent child exit tolerates a nonzero killer result",
  acceptsConcurrentWindowsExit,
);
test(
  "POSIX shutdown preserves the exact owned-group signal sequence",
  preservesPosixSignalSequence,
);
test(
  "POSIX ignores only missing groups and retains final deadlines",
  preservesPosixFailures,
);
test(
  "help and invalid arguments precede preflight",
  admitsArgumentsBeforePreflight,
);
test("unsafe preflight never starts or stops services", rejectsUnsafePreflight);
test(
  "POSIX service starts register detached owned groups",
  startsDetachedPosixGroup,
);
test("interrupted builds remain owned through cleanup", cleansInterruptedBuild);
test(
  "ready services preserve exact launches and cleanup ownership",
  supervisesOwnedServices,
);
test(
  "service failure still cleans the surviving sibling",
  cleansAfterServiceFailure,
);
test(
  "cleanup failure does not abandon other registered children",
  continuesAfterCleanupFailure,
);
