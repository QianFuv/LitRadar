/** Verify profile interruption propagates and cleans only the owned run resources. */
import assert from "node:assert/strict";
import childProcess from "node:child_process";
import { EventEmitter } from "node:events";
import fs from "node:fs";
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
