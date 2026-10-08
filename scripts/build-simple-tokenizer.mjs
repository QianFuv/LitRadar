/** Build verified static Simple inputs and optional legacy compatibility oracles. */

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { copyFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REVISION = "45db071ba8043ffe8a2e5dfe41f9d68fb477576c";
const SOURCE_SHA256 =
  "d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61";
const HEADERS = ["sqlite3-binding.h", "sqlite3ext.h"];

/** Hash actual native bytes before accepting an input identity. */
function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/** Run one native build command and reject failed execution. */
async function run(command, args, root = ROOT) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: root,
      stdio: "inherit",
      windowsHide: true,
      timeout: 300000,
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} failed with exit code ${code}`));
    });
  });
}

/** Build against the SQLite driver's headers with exactly Go's target compiler. */
export async function buildSimple(root = ROOT, hasCompatibilityOracle = false) {
  const buildRoot = path.join(root, "target/simple-tokenizer-build");
  const outputRoot = path.join(root, "target/simple-tokenizer");
  await mkdir(buildRoot, { recursive: true });
  await mkdir(outputRoot, { recursive: true });
  const archivePath = path.join(buildRoot, "simple-source.tar.gz");
  const downloadPath = `${archivePath}.download`;
  let shouldPublishArchive = false;
  let archive;
  try {
    archive = await readFile(archivePath);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    await run(
      "curl",
      [
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--retry",
        "2",
        "--connect-timeout",
        "15",
        "--max-time",
        "60",
        "--output",
        downloadPath,
        `https://codeload.github.com/wangfenjin/simple/tar.gz/${REVISION}`,
      ],
      root,
    );
    archive = await readFile(downloadPath);
    shouldPublishArchive = true;
  }
  if (createHash("sha256").update(archive).digest("hex") !== SOURCE_SHA256) {
    throw new Error("Tokenizer source checksum mismatch");
  }
  if (shouldPublishArchive) await rename(downloadPath, archivePath);
  await run("tar", ["-xzf", archivePath, "-C", buildRoot], root);
  const source = path.join(buildRoot, `simple-${REVISION}`);
  const driver = path.join(root, "third_party/go-sqlite3");
  const target = JSON.parse(
    execFileSync("go", ["env", "-json", "CC", "CXX", "GOOS", "GOARCH"], {
      cwd: root,
      encoding: "utf8",
      timeout: 60000,
      windowsHide: true,
    }),
  );
  assert(
    ["windows", "linux"].includes(target.GOOS),
    "Unsupported native Simple target",
  );
  const compiler = execFileSync(target.CXX, ["--version"], {
    encoding: "utf8",
    timeout: 10000,
    windowsHide: true,
  });
  const compilerTarget = execFileSync(target.CXX, ["-dumpmachine"], {
    encoding: "utf8",
    windowsHide: true,
    timeout: 10000,
  }).trim();
  const architecture = target.GOARCH === "amd64" ? /^x86_64-/ : /^aarch64-/;
  assert(
    architecture.test(compilerTarget),
    "C++ compiler does not match Go target architecture",
  );
  assert.equal(
    /mingw|windows/i.test(compilerTarget),
    target.GOOS === "windows",
    "C++ compiler does not match Go target operating system",
  );
  const buildDirectory = path.join(
    buildRoot,
    `static-${target.GOOS}-${target.GOARCH}`,
  );
  await run(
    "cmake",
    [
      "-S",
      path.join(root, "third_party/simple-static"),
      "-B",
      buildDirectory,
      ...(process.platform === "win32" ? ["-G", "Ninja"] : []),
      "-DCMAKE_BUILD_TYPE=Release",
      `-DSIMPLE_SOURCE_DIR=${source}`,
      `-DSQLITE_DRIVER_DIR=${driver}`,
      `-DCMAKE_CXX_COMPILER=${target.CXX}`,
      `-DCMAKE_ARCHIVE_OUTPUT_DIRECTORY=${outputRoot}`,
    ],
    root,
  );
  await run(
    "cmake",
    [
      "--build",
      buildDirectory,
      "--config",
      "Release",
      "--target",
      "simple",
      "--parallel",
      "2",
    ],
    root,
  );
  const identity = {
    sourceSha256: SOURCE_SHA256,
    revision: REVISION,
    target,
    compiler,
    compilerTarget,
    archiveSha256: digest(await readFile(path.join(outputRoot, "libsimple.a"))),
    headers: Object.fromEntries(
      await Promise.all(
        HEADERS.map(async (name) => [
          name,
          digest(await readFile(path.join(driver, name))),
        ]),
      ),
    ),
    adapterSha256: digest(
      await readFile(
        path.join(root, "third_party/simple-static/CMakeLists.txt"),
      ),
    ),
  };
  await writeFile(
    path.join(outputRoot, "inputs.json"),
    JSON.stringify(identity, null, 2) + "\n",
  );
  if (hasCompatibilityOracle) {
    const oracleRoot = path.join(root, "target/simple-tokenizer-oracle");
    await mkdir(oracleRoot, { recursive: true });
    const filename = target.GOOS === "windows" ? "simple.dll" : "libsimple.so";
    if (target.GOOS === "windows") {
      const previous = path.join(root, "libs/simple/windows/simple.dll");
      assert.equal(
        digest(await readFile(previous)),
        "27c700ca34cd5935ff934459f1f9c107cdefef7e54250de5a1c6646e05f21a4f",
      );
      await copyFile(previous, path.join(oracleRoot, filename));
    } else {
      const oracleBuild = path.join(buildRoot, `oracle-${target.GOARCH}`);
      await run(
        "cmake",
        [
          "-S",
          source,
          "-B",
          oracleBuild,
          "-DCMAKE_BUILD_TYPE=Release",
          "-DSIMPLE_WITH_JIEBA=OFF",
          "-DBUILD_SQLITE3=OFF",
          "-DBUILD_TEST_EXAMPLE=OFF",
          "-DBUILD_STATIC=OFF",
          `-DCMAKE_C_COMPILER=${target.CC}`,
          `-DCMAKE_CXX_COMPILER=${target.CXX}`,
          `-DCMAKE_LIBRARY_OUTPUT_DIRECTORY=${oracleRoot}`,
        ],
        root,
      );
      await run(
        "cmake",
        ["--build", oracleBuild, "--target", "simple", "--parallel", "2"],
        root,
      );
    }
    await writeFile(
      path.join(oracleRoot, "inputs.json"),
      JSON.stringify(
        {
          target,
          source: target.GOOS === "windows" ? "v0.7.1 bundled DLL" : REVISION,
          filename,
          sha256: digest(await readFile(path.join(oracleRoot, filename))),
        },
        null,
        2,
      ) + "\n",
    );
  }
  return identity;
}

/** Preserve caller flags and include verified archive/header bytes in Go's compilation cache key. */
export function simpleBuildEnvironment(root = ROOT, environment = process.env) {
  const output = path.join(root, "target/simple-tokenizer");
  const identity = JSON.parse(
    readFileSync(path.join(output, "inputs.json"), "utf8"),
  );
  assert.equal(
    identity.sourceSha256,
    SOURCE_SHA256,
    "Rebuild changed Simple source",
  );
  assert.equal(identity.revision, REVISION, "Rebuild changed Simple revision");
  assert.equal(
    digest(
      readFileSync(path.join(root, "third_party/simple-static/CMakeLists.txt")),
    ),
    identity.adapterSha256,
    "Rebuild changed Simple adapter",
  );
  const target = JSON.parse(
    execFileSync("go", ["env", "-json", "CC", "CXX", "GOOS", "GOARCH"], {
      cwd: root,
      env: environment,
      encoding: "utf8",
      windowsHide: true,
      timeout: 60000,
    }),
  );
  assert.deepEqual(
    target,
    identity.target,
    "Rebuild changed native target or compiler",
  );
  assert.equal(
    digest(readFileSync(path.join(output, "libsimple.a"))),
    identity.archiveSha256,
    "Rebuild changed Simple archive",
  );
  for (const name of HEADERS) {
    assert.equal(
      digest(readFileSync(path.join(root, "third_party/go-sqlite3", name))),
      identity.headers[name],
      "Rebuild changed SQLite headers",
    );
  }
  const marker = digest(JSON.stringify(identity));
  const flags = (environment.CGO_CFLAGS ?? "")
    .replace(/(?:^|\s)-DLITRADAR_SIMPLE_INPUT_[A-Fa-f0-9]+(?:=1)?/g, "")
    .trim();
  return {
    ...environment,
    CGO_CFLAGS: `${flags} -DLITRADAR_SIMPLE_INPUT_${marker}=1`.trim(),
  };
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  assert(
    process.argv
      .slice(2)
      .every((argument) => argument === "--compatibility-oracle"),
    "Unknown native build option",
  );
  await buildSimple(ROOT, process.argv.includes("--compatibility-oracle"));
}
