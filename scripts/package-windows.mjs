/** Build a Windows x64 distribution from an exact source commit and pinned native runtimes. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseVersion } from "./release-version.mjs";

const toolsRoot = fileURLToPath(new URL("..", import.meta.url));
const [sourceArgument, inputVersion, commit] = process.argv.slice(2);
assert.equal(
  process.platform,
  "win32",
  "Windows packaging requires a native Windows runner",
);
assert.match(commit, /^[a-f0-9]{40}$/);
const source = path.resolve(sourceArgument);
const version = parseVersion(inputVersion);
const output = path.resolve("release-results/windows");
const name = `litradar_${version}_windows_amd64`;
const directory = path.join(output, "packages", name);
const assets = path.join(output, "assets");
const dependencies = JSON.parse(
  fs.readFileSync(
    path.join(toolsRoot, "docs/third-party/windows-release.json"),
    "utf8",
  ),
);

/** Execute a bounded build tool, never interpreting arguments as shell code. */
function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: source,
    encoding: "utf8",
    windowsHide: true,
    timeout: 900000,
    ...options,
  });
}
assert.equal(run("git", ["rev-parse", "HEAD"]).trim(), commit);
assert.equal(
  run("git", ["status", "--porcelain", "--untracked-files=no"]).trim(),
  "",
  "Source checkout must be clean",
);
assert.equal(
  parseVersion(fs.readFileSync(path.join(source, "VERSION"), "utf8")),
  version,
);
assert(!fs.existsSync(directory), "Package directory already exists");
fs.mkdirSync(directory, { recursive: true });
fs.mkdirSync(assets, { recursive: true });

/** Download immutable archive bytes once and require the recorded digest before extraction. */
async function dependencyArchive(key, definition) {
  const cache = path.resolve("release-results/downloads");
  fs.mkdirSync(cache, { recursive: true });
  const archive = path.join(cache, `${key}-${definition.sha256}.zip`);
  if (!fs.existsSync(archive)) {
    const response = await fetch(definition.url, {
      signal: AbortSignal.timeout(300000),
    });
    assert(response.ok, `Download failed: ${key} (${response.status})`);
    fs.writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
  }
  assert.equal(
    createHash("sha256").update(fs.readFileSync(archive)).digest("hex"),
    definition.sha256,
    `${key} archive digest differs`,
  );
  return archive;
}

const environment = {
  ...process.env,
  CGO_ENABLED: "1",
  GOWORK: "off",
  GOENV: "off",
  GOFLAGS: "",
  GOTOOLCHAIN: "go1.27.1",
  GOOS: "windows",
  GOARCH: "amd64",
};
run(
  "go",
  [
    "build",
    "-mod=readonly",
    "-trimpath",
    "-tags",
    "sqlite_fts5,sqlite_dbstat",
    "-ldflags",
    "-linkmode external -extldflags -static",
    "-o",
    path.join(directory, "litradar.exe"),
    "./cmd/litradar",
  ],
  { env: environment, stdio: "inherit" },
);
for (const [from, to] of [
  ["app/out", "web"],
  ["assets/meta", "assets/meta"],
  ["libs/simple/windows/simple.dll", "simple.dll"],
  ["docs/third-party", "licenses"],
  ["LICENSE", "LICENSE"],
]) {
  fs.cpSync(path.join(source, from), path.join(directory, to), {
    recursive: true,
  });
}
fs.copyFileSync(
  path.join(toolsRoot, "docs/third-party/windows-release.json"),
  path.join(directory, "licenses/windows-release.json"),
);
fs.copyFileSync(
  path.join(toolsRoot, "docs/third-party/NOTICE.txt"),
  path.join(directory, "licenses/NOTICE.txt"),
);
const inventory = path.join(directory, "licenses/go-inventory");
fs.mkdirSync(inventory, { recursive: true });
fs.writeFileSync(
  path.join(inventory, "binary-modules.txt"),
  run("go", ["version", "-m", path.join(directory, "litradar.exe")], {
    env: environment,
  }),
);
const modules = run(
  "go",
  [
    "list",
    "-mod=readonly",
    "-m",
    "-f",
    "{{.Path}}|{{if .Replace}}{{.Replace.Dir}}{{else}}{{.Dir}}{{end}}",
    "all",
  ],
  { env: environment },
);
fs.writeFileSync(path.join(inventory, "modules.txt"), modules);
for (const line of modules.trim().split(/\r?\n/)) {
  const [moduleName, moduleDirectory] = line.split("|");
  if (!moduleDirectory) continue;
  for (const filename of fs.readdirSync(moduleDirectory)) {
    if (
      !/^(license|notice|copying)/i.test(filename) ||
      !fs.statSync(path.join(moduleDirectory, filename)).isFile()
    )
      continue;
    const destination = path.join(inventory, "licenses", moduleName);
    fs.mkdirSync(destination, { recursive: true });
    fs.copyFileSync(
      path.join(moduleDirectory, filename),
      path.join(destination, filename),
    );
  }
}
fs.copyFileSync(
  path.join(
    run("go", ["env", "GOROOT"], { env: environment }).trim(),
    "LICENSE",
  ),
  path.join(inventory, "Go-LICENSE"),
);
run("tar", [
  "-xf",
  await dependencyArchive("obscura", dependencies.obscura),
  "-C",
  directory,
]);
const nativeDirectory = path.join(directory, "native");
fs.mkdirSync(nativeDirectory);
run("tar", [
  "-xf",
  await dependencyArchive("poppler", dependencies.poppler),
  "-C",
  nativeDirectory,
]);
const poppler = path.join(nativeDirectory, dependencies.poppler.directory);
const popplerBin = path.join(poppler, "Library/bin");
for (const filename of [
  "msvcp140.dll",
  "vcruntime140.dll",
  "vcruntime140_1.dll",
]) {
  fs.copyFileSync(
    path.join(popplerBin, filename),
    path.join(directory, filename),
  );
}
fs.writeFileSync(path.join(directory, "VERSION"), version + "\n");
const provenance = {
  version,
  sourceCommit: commit,
  toolingCommit: run("git", ["rev-parse", "HEAD"], { cwd: toolsRoot }).trim(),
  dependencies,
};
fs.writeFileSync(
  path.join(directory, "build.json"),
  JSON.stringify(provenance, null, 2) + "\n",
);
fs.copyFileSync(
  path.join(toolsRoot, "docs/operations/releases.md"),
  path.join(directory, "README.md"),
);
fs.writeFileSync(
  path.join(directory, "run.ps1"),
  `$ErrorActionPreference = 'Stop'
$previousPath = $env:PATH
$previousObscura = $env:LITRADAR_OBSCURA_PATH
$previousPdf = $env:LITRADAR_PDFTOTEXT_PATH
Push-Location -LiteralPath $PSScriptRoot
try {
  $env:PATH = "$PSScriptRoot;" + (Join-Path $PSScriptRoot 'native/${dependencies.poppler.directory}/Library/bin') + ";$previousPath"
  if (-not $env:LITRADAR_OBSCURA_PATH) { $env:LITRADAR_OBSCURA_PATH = Join-Path $PSScriptRoot 'obscura.exe' }
  if (-not $env:LITRADAR_PDFTOTEXT_PATH) { $env:LITRADAR_PDFTOTEXT_PATH = Join-Path $PSScriptRoot 'native/${dependencies.poppler.directory}/Library/bin/pdftotext.exe' }
  & (Join-Path $PSScriptRoot 'litradar.exe') @args
  exit $LASTEXITCODE
} finally {
  Pop-Location
  $env:PATH = $previousPath
  $env:LITRADAR_OBSCURA_PATH = $previousObscura
  $env:LITRADAR_PDFTOTEXT_PATH = $previousPdf
}
`,
);
const archive = path.join(assets, `${name}.zip`);
run("tar", ["-a", "-cf", archive, "-C", path.dirname(directory), name]);
run(
  process.execPath,
  [path.join(toolsRoot, "scripts/smoke-windows.mjs"), archive, version, commit],
  { cwd: toolsRoot, stdio: "inherit" },
);
const checksum = `${createHash("sha256").update(fs.readFileSync(archive)).digest("hex")}  ${name}.zip\n`;
fs.writeFileSync(`${archive}.sha256`, checksum);
console.log(archive);
