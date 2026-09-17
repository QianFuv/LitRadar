/** Build the pinned non-Jieba SQLite extension for Linux development and CI. */

import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { mkdir, readFile, rename } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REVISION = "45db071ba8043ffe8a2e5dfe41f9d68fb477576c";
const SOURCE_SHA256 =
  "d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61";
const BUILD_ROOT = path.join(ROOT, "target", "simple-tokenizer-build");
const OUTPUT_ROOT = path.join(ROOT, "target", "simple-tokenizer");

/** Run one native build command and reject failed execution. */
async function run(command, args) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: ROOT,
      stdio: "inherit",
      windowsHide: true,
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} failed with exit code ${code}`));
    });
  });
}

/** Verify source bytes before configuring a native-target build without optional dictionaries. */
async function main() {
  if (process.platform !== "linux") {
    throw new Error(
      "This build helper targets Linux; Windows x64 uses the checked-in simple.dll",
    );
  }
  await mkdir(BUILD_ROOT, { recursive: true });
  await mkdir(OUTPUT_ROOT, { recursive: true });
  const archivePath = path.join(BUILD_ROOT, "simple-source.tar.gz");
  const downloadPath = `${archivePath}.download`;
  let shouldPublishArchive = false;
  let archive;
  try {
    archive = await readFile(archivePath);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    await run("curl", [
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
    ]);
    archive = await readFile(downloadPath);
    shouldPublishArchive = true;
  }
  if (createHash("sha256").update(archive).digest("hex") !== SOURCE_SHA256) {
    throw new Error("Tokenizer source checksum mismatch");
  }
  if (shouldPublishArchive) await rename(downloadPath, archivePath);
  await run("tar", ["-xzf", archivePath, "-C", BUILD_ROOT]);
  const buildDirectory = path.join(BUILD_ROOT, "build");
  await run("cmake", [
    "-S",
    path.join(BUILD_ROOT, `simple-${REVISION}`),
    "-B",
    buildDirectory,
    "-DCMAKE_BUILD_TYPE=Release",
    "-DSIMPLE_WITH_JIEBA=OFF",
    "-DBUILD_SQLITE3=OFF",
    "-DBUILD_TEST_EXAMPLE=OFF",
    "-DBUILD_STATIC=OFF",
    `-DCMAKE_LIBRARY_OUTPUT_DIRECTORY=${OUTPUT_ROOT}`,
  ]);
  await run("cmake", [
    "--build",
    buildDirectory,
    "--config",
    "Release",
    "--target",
    "simple",
    "--parallel",
    "2",
  ]);
}

await main();
