/** Verify server source checks stay inside the root module after the client split. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const FORMAT_SCRIPT = path.resolve("scripts/check-go-format.mjs");

/** Run a bounded local tool inside a synthetic workspace. */
function runTool(command, args, root) {
  return spawnSync(command, args, {
    cwd: root,
    env: { ...process.env, GOWORK: "off", GOENV: "off", GOFLAGS: "" },
    encoding: "utf8",
    timeout: 60000,
    windowsHide: true,
  });
}

test("server formatting includes the moved entry and excludes the nested client module", async (context) => {
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-server-check-"),
  );
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  for (const directory of [
    "cmd/fixture",
    "internal/example",
    "apps/server",
    "apps/client",
  ]) {
    await fs.mkdir(path.join(root, directory), { recursive: true });
  }
  await fs.writeFile(
    path.join(root, "go.mod"),
    "module example.com/server\n\ngo 1.27.3\n",
  );
  await fs.writeFile(
    path.join(root, "apps/client/go.mod"),
    "module example.com/client\n\ngo 1.27.3\n",
  );
  await fs.writeFile(
    path.join(root, "apps/client/broken.go"),
    "deliberately invalid client source",
  );
  const server = path.join(root, "apps/server/main.go");
  await fs.writeFile(server, "package main\n\nfunc main() {}\n");
  let result = runTool(process.execPath, [FORMAT_SCRIPT], root);
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const graph = runTool("go", ["list", "./..."], root);
  assert.ifError(graph.error);
  assert.equal(graph.status, 0, graph.stderr);
  assert.equal(graph.stdout.trim(), "example.com/server/apps/server");
  await fs.writeFile(server, "package main\nfunc main(){ }\n");
  result = runTool(process.execPath, [FORMAT_SCRIPT], root);
  assert.ifError(result.error);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /apps[\\/]server[\\/]main.go/);
});

/** Extract the shipped launcher template while supplying only its pinned dependency directory. */
async function launcherTemplate(filename, launcher) {
  const source = await fs.readFile(filename, "utf8");
  const marker = `path.join(directory, "${launcher}"),`;
  const start = source.indexOf(marker);
  assert(start >= 0);
  const templateStart = source.indexOf("`", start);
  const templateEnd = source.indexOf("`", templateStart + 1);
  return new Function(
    "dependencies",
    "return " + source.slice(templateStart, templateEnd + 1),
  )({
    poppler: { directory: "fixture-poppler" },
  });
}

test(
  "Windows server launcher preserves command position, version, and explicit project roots",
  { skip: process.platform !== "win32" },
  async (context) => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-launcher-"));
    context.after(() => fs.rm(root, { recursive: true, force: true }));
    let launcher = await launcherTemplate(
      "scripts/package-windows.mjs",
      "run.ps1",
    );
    const invocation = "  & (Join-Path $PSScriptRoot 'litradar.exe') @args";
    assert(launcher.includes(invocation));
    launcher = launcher.replace(
      invocation,
      "  [Console]::Write((ConvertTo-Json -InputObject @($args) -Compress)); $global:LASTEXITCODE = 0",
    );
    const filename = path.join(root, "run.ps1");
    await fs.writeFile(filename, launcher);
    const powershell = path.join(
      process.env.SystemRoot,
      "System32/WindowsPowerShell/v1.0/powershell.exe",
    );
    for (const [args, expected] of [
      [["--version"], ["--version"]],
      [
        ["serve", "--host", "127.0.0.1"],
        ["serve", "--host", "127.0.0.1", "--project-root", root],
      ],
      [
        ["admin", "bootstrap", "--project-root", "explicit root"],
        ["admin", "bootstrap", "--project-root", "explicit root"],
      ],
      [["openapi"], ["openapi"]],
    ]) {
      const result = runTool(
        powershell,
        [
          "-NoProfile",
          "-ExecutionPolicy",
          "Bypass",
          "-File",
          filename,
          ...args,
        ],
        root,
      );
      assert.ifError(result.error);
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(JSON.parse(result.stdout), expected);
    }
  },
);

test(
  "Linux server launcher preserves commands and explicit project roots",
  { skip: process.platform === "win32" },
  async (context) => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-launcher-"));
    context.after(() => fs.rm(root, { recursive: true, force: true }));
    let launcher = await launcherTemplate(
      "scripts/package-release.mjs",
      "run.sh",
    );
    const invocation = 'exec "$root/litradar" "$@"';
    assert(launcher.includes(invocation));
    launcher = launcher.replace(invocation, `printf '%s\\n' "$@"`);
    const filename = path.join(root, "run.sh");
    await fs.writeFile(filename, launcher);
    for (const [args, expected] of [
      [["--version"], ["--version"]],
      [["serve"], ["serve", "--project-root", root]],
      [
        ["admin", "bootstrap", "--project-root", "explicit root"],
        ["admin", "bootstrap", "--project-root", "explicit root"],
      ],
    ]) {
      const result = runTool("sh", [filename, ...args], root);
      assert.ifError(result.error);
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(result.stdout.trimEnd().split("\n"), expected);
    }
  },
);
