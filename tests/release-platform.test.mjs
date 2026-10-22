/** Verify unsupported release targets fail before packaging can create or extract files. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import { assertReleasePlatform } from "../scripts/release-platform.mjs";

test("release targets are exactly Windows/Linux amd64, independent of source support", () => {
  for (const operatingSystem of ["linux", "windows"]) {
    assert.doesNotThrow(() =>
      assertReleasePlatform(operatingSystem, "amd64", operatingSystem),
    );
    for (const architecture of ["arm64", "386", "arm", undefined]) {
      assert.throws(() =>
        assertReleasePlatform(operatingSystem, architecture, operatingSystem),
      );
    }
  }
  for (const operatingSystem of ["android", "darwin", "freebsd"]) {
    assert.throws(() =>
      assertReleasePlatform(operatingSystem, "amd64", operatingSystem),
    );
    assert.throws(() =>
      assertReleasePlatform(operatingSystem, "amd64", "linux"),
    );
  }
  assert.throws(() => assertReleasePlatform("linux", "amd64", "windows"));
});

/** Run the real CLI with only its external read command replaced, never invoking Docker or Go. */
async function rejectedPackage(context, kind, operatingSystem, architecture) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-platform-"));
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  const log = path.join(root, "commands.jsonl");
  const preload = path.join(root, "mock-tools.mjs");
  const identity =
    kind === "linux"
      ? [{ Os: operatingSystem, Architecture: architecture }]
      : { GOOS: operatingSystem, GOARCH: architecture };
  await fs.writeFile(
    preload,
    `
import childProcess from 'node:child_process';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
${kind === "windows" ? "Object.defineProperty(process, 'platform', { value: 'win32' });" : ""}
childProcess.execFileSync = (command, args) => {
  fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify([command, ...args]) + '\\n');
  if (command === ${JSON.stringify(kind === "linux" ? "docker" : "go")} && args[0] === ${JSON.stringify(kind === "linux" ? "image" : "env")}) return ${JSON.stringify(JSON.stringify(identity))};
  throw new Error('Unexpected packaging side effect: ' + command);
};
syncBuiltinESMExports();
`,
  );
  const script = path.resolve(
    `scripts/package-${kind === "linux" ? "release" : "windows"}.mjs`,
  );
  const args =
    kind === "linux"
      ? ["candidate", "amd64", "0.2.4"]
      : [root, "0.2.4", "a".repeat(40), "--local-smoke"];
  const result = spawnSync(
    process.execPath,
    ["--import", pathToFileURL(preload).href, script, ...args],
    {
      cwd: root,
      encoding: "utf8",
      timeout: 15000,
      windowsHide: true,
    },
  );
  assert.equal(result.status, 1, result.stderr);
  assert.match(
    result.stderr,
    /Release (operating system differs|architecture must be amd64)/,
  );
  const calls = (await fs.readFile(log, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.deepEqual(
    calls,
    kind === "linux"
      ? [["docker", "image", "inspect", "candidate"]]
      : [["go", "env", "-json", "GOOS", "GOARCH"]],
  );
  for (const directory of ["release-results", "test-results", "target"]) {
    await assert.rejects(fs.stat(path.join(root, directory)), {
      code: "ENOENT",
    });
  }
}

for (const kind of ["linux", "windows"]) {
  for (const [operatingSystem, architecture] of [
    [kind, "arm64"],
    ["darwin", "amd64"],
    ["android", "amd64"],
  ]) {
    test(`${kind} packager rejects actual ${operatingSystem}/${architecture} before side effects`, async (context) => {
      await rejectedPackage(context, kind, operatingSystem, architecture);
    });
  }
}
