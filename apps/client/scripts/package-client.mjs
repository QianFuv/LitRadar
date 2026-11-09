/**
 * Package client builds under the release naming scheme and stage them for release.
 *
 * Every artifact is named `litradar-<component>_<version>_<os>_<arch>.<ext>`; `package` writes
 * that name into `apps/client/bin`, and `stage` copies the same file into the CI results.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseVersion } from "../../../scripts/release-version.mjs";

const REPOSITORY_ROOT = fileURLToPath(new URL("../../../", import.meta.url));

/** Return the single supported artifact for each client platform. */
export function clientAssetName(version, platform) {
  parseVersion(version);
  const names = {
    web: `litradar-client-web_${version}_linux_amd64.tar.gz`,
    windows: `litradar-client_${version}_windows_amd64.zip`,
    android: `litradar-client_${version}_android_arm64.apk`,
  };
  assert(Object.hasOwn(names, platform), "Unsupported client platform");
  return names[platform];
}

/**
 * Return the local name of the debuggable APK, which is never released.
 *
 * @param {string} version - Release version from VERSION.
 * @returns {string} The release APK name with a `_debug` variant suffix.
 */
export function debugApkName(version) {
  return clientAssetName(version, "android").replace(/\.apk$/, "_debug.apk");
}

/**
 * Archive one executable and LICENSE in a folder named after the archive.
 *
 * @param {string} archive - Destination `.zip` or `.tar.gz` path.
 * @param {string} executable - Built executable to include.
 * @param {string} root - Repository root holding LICENSE.
 */
function archiveFolder(archive, executable, root) {
  const name = path.basename(archive).replace(/\.(zip|tar\.gz)$/, "");
  const staging = fs.mkdtempSync(path.join(path.dirname(archive), "package-"));
  try {
    const folder = path.join(staging, name);
    fs.mkdirSync(folder);
    const target = path.join(folder, path.basename(executable));
    fs.copyFileSync(executable, target);
    fs.chmodSync(target, 0o755);
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(folder, "LICENSE"));
    fs.rmSync(archive, { force: true });
    // ponytail: the Windows tar drops the executable bit; release web archives are built on Linux.
    const isWindows = process.platform === "win32";
    assert(
      isWindows || archive.endsWith(".tar.gz"),
      "Zip archives need the Windows tar",
    );
    execFileSync(
      isWindows
        ? path.join(process.env.SystemRoot, "System32", "tar.exe")
        : "tar",
      [isWindows ? "-acf" : "-czf", archive, "-C", staging, name],
      { windowsHide: true, timeout: 300000 },
    );
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

/**
 * Write the release-named package of one platform into apps/client/bin.
 *
 * @param {string} platform - `web`, `windows`, `android` or `android-debug`.
 * @param {string} version - Release version from VERSION.
 * @param {string} root - Repository root holding LICENSE and apps/client.
 * @returns {string} Path of the written package.
 */
export function packageClient(platform, version, root = REPOSITORY_ROOT) {
  const bin = path.join(root, "apps/client/bin");
  const apks = path.join(
    root,
    "apps/client/build/android/app/build/outputs/apk",
  );
  fs.mkdirSync(bin, { recursive: true });
  if (platform === "android-debug") {
    const target = path.join(bin, debugApkName(version));
    fs.copyFileSync(path.join(apks, "debug/app-debug.apk"), target);
    return target;
  }
  const target = path.join(bin, clientAssetName(version, platform));
  if (platform === "android") {
    fs.copyFileSync(path.join(apks, "release/app-release.apk"), target);
  } else {
    const executable =
      platform === "web" ? "litradar-client-web" : "litradar-client.exe";
    archiveFolder(target, path.join(bin, executable), root);
  }
  return target;
}

/** Copy only the Task contract's exact output, rejecting empty or linked files. */
export function stageClient(
  platform,
  version,
  root = ".",
  output = "release-results/client",
) {
  const name = clientAssetName(version, platform);
  const source = path.join(root, "apps/client/bin", name);
  const entry = fs.lstatSync(source);
  assert(
    entry.isFile() && entry.size > 0,
    "Client output must be a nonempty regular file",
  );
  const directory = path.join(output, "assets");
  fs.mkdirSync(directory, { recursive: true });
  fs.copyFileSync(
    source,
    path.join(directory, name),
    fs.constants.COPYFILE_EXCL,
  );
}

/** Read the release version from the repository VERSION file. */
export function repositoryVersion(root = REPOSITORY_ROOT) {
  return parseVersion(fs.readFileSync(path.join(root, "VERSION"), "utf8"));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [command, platform, inputVersion] = process.argv.slice(2);
  if (command === "package") {
    packageClient(platform, repositoryVersion());
  } else {
    assert.equal(
      command,
      "stage",
      "Usage: package <platform> | stage <platform> <version>",
    );
    stageClient(platform, parseVersion(inputVersion));
  }
}
