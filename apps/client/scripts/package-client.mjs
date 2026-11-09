/** Package client builds and stage them for release under immutable, versioned names. */
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
    web: `litradar-client-web_${version}_linux_amd64`,
    windows: `litradar-client_${version}_windows_amd64.zip`,
    android: `litradar-client_${version}_android_arm64.apk`,
  };
  assert(Object.hasOwn(names, platform), "Unsupported client platform");
  return names[platform];
}

/**
 * Zip the Windows executable as a portable folder named after the release asset.
 *
 * @param {string} version - Release version the archive is named after.
 * @param {string} root - Repository root holding LICENSE and apps/client/bin.
 * @returns {string} Path of the written archive.
 */
export function packageWindowsPortable(version, root = REPOSITORY_ROOT) {
  assert.equal(
    process.platform,
    "win32",
    "Windows packaging requires the Windows tar",
  );
  const bin = path.join(root, "apps/client/bin");
  const archive = path.join(bin, clientAssetName(version, "windows"));
  const name = path.basename(archive, ".zip");
  const staging = fs.mkdtempSync(path.join(bin, "package-"));
  try {
    const folder = path.join(staging, name);
    fs.mkdirSync(folder);
    fs.copyFileSync(
      path.join(bin, "litradar-client.exe"),
      path.join(folder, "litradar-client.exe"),
    );
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(folder, "LICENSE"));
    fs.rmSync(archive, { force: true });
    execFileSync(
      path.join(process.env.SystemRoot, "System32", "tar.exe"),
      ["-a", "-cf", archive, "-C", staging, name],
      { windowsHide: true, timeout: 300000 },
    );
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
  return archive;
}

/** Copy only the Task contract's exact output, rejecting empty or linked files. */
export function stageClient(
  platform,
  version,
  root = ".",
  output = "release-results/client",
) {
  const sources = {
    web: "litradar-client-web",
    windows: clientAssetName(version, "windows"),
    android: "litradar-client-android-arm64.apk",
  };
  const name = clientAssetName(version, platform);
  const source = path.join(root, "apps/client/bin", sources[platform]);
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
  if (platform === "web") fs.chmodSync(path.join(directory, name), 0o755);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [command, platform, inputVersion] = process.argv.slice(2);
  if (command === "package") {
    assert.equal(platform, "windows", "Only Windows has a package step");
    packageWindowsPortable(
      parseVersion(
        fs.readFileSync(path.join(REPOSITORY_ROOT, "VERSION"), "utf8"),
      ),
    );
  } else {
    assert.equal(
      command,
      "stage",
      "Usage: package <platform> | stage <platform> <version>",
    );
    stageClient(platform, parseVersion(inputVersion));
  }
}
