/**
 * Derive client build metadata from the repository VERSION file, the only place a release
 * version is written.
 *
 * Usage (from apps/client):
 * - `node scripts/client-version.mjs name` prints the MAJOR.MINOR.PATCH version name.
 * - `node scripts/client-version.mjs windows-resources <directory>` writes `info.json` and
 *   `wails.exe.manifest` with the release version into `<directory>` for `wails3 generate syso`.
 *   The tracked `build/windows/info.json` carries no version fields, and the tracked
 *   `build/windows/wails.exe.manifest` uses the `{{FILE_VERSION}}` placeholder as the
 *   application assembly version.
 *
 * The Android Gradle build derives the same values in `build/android/app/build.gradle`.
 */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseVersion } from "../../../scripts/release-version.mjs";

export const MANIFEST_VERSION_PLACEHOLDER = "{{FILE_VERSION}}";

const VERSION_FILE = new URL("../../../VERSION", import.meta.url);
const WINDOWS_BUILD_DIRECTORY = new URL("../build/windows/", import.meta.url);

/**
 * Derive the client version identifiers from the VERSION file contents.
 *
 * @param versionText - The contents of the repository VERSION file.
 * @returns The `versionName` (MAJOR.MINOR.PATCH), the Android `versionCode`
 *   (major * 10000 + minor * 100 + patch), and the four-part Windows `fileVersion`.
 */
export function clientVersions(versionText) {
  const versionName = parseVersion(versionText);
  const [major, minor, patch] = versionName.split(".").map(Number);
  assert(
    minor < 100 && patch < 100,
    "VERSION minor and patch must be below 100 to form the Android versionCode",
  );
  assert(
    major <= 65535,
    "VERSION major must not exceed 65535 to fit a Windows file version",
  );
  return {
    versionName,
    versionCode: major * 10000 + minor * 100 + patch,
    fileVersion: `${versionName}.0`,
  };
}

/**
 * Render the Windows version resource and application manifest for a release.
 *
 * @param infoText - The version-free `info.json` template.
 * @param manifestText - The manifest template containing {@link MANIFEST_VERSION_PLACEHOLDER}.
 * @param versions - The identifiers returned by {@link clientVersions}.
 * @returns The rendered `info` JSON and `manifest` XML text.
 */
export function windowsResources(infoText, manifestText, versions) {
  const info = JSON.parse(infoText);
  info.fixed = {
    ...info.fixed,
    file_version: versions.fileVersion,
    product_version: versions.fileVersion,
  };
  for (const strings of Object.values(info.info)) {
    strings.FileVersion = versions.versionName;
    strings.ProductVersion = versions.versionName;
  }
  assert(
    manifestText.includes(MANIFEST_VERSION_PLACEHOLDER),
    `The Windows manifest template must contain ${MANIFEST_VERSION_PLACEHOLDER}`,
  );
  return {
    info: `${JSON.stringify(info, null, 2)}\n`,
    manifest: manifestText.replaceAll(
      MANIFEST_VERSION_PLACEHOLDER,
      versions.fileVersion,
    ),
  };
}

/**
 * Write the rendered Windows resources for the repository VERSION into a directory.
 *
 * @param directory - The output directory, created when missing.
 */
function writeWindowsResources(directory) {
  assert(directory, "windows-resources requires an output directory");
  const resources = windowsResources(
    fs.readFileSync(new URL("info.json", WINDOWS_BUILD_DIRECTORY), "utf8"),
    fs.readFileSync(
      new URL("wails.exe.manifest", WINDOWS_BUILD_DIRECTORY),
      "utf8",
    ),
    clientVersions(fs.readFileSync(VERSION_FILE, "utf8")),
  );
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, "info.json"), resources.info);
  fs.writeFileSync(
    path.join(directory, "wails.exe.manifest"),
    resources.manifest,
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [command, directory] = process.argv.slice(2);
  if (command === "name") {
    process.stdout.write(
      clientVersions(fs.readFileSync(VERSION_FILE, "utf8")).versionName,
    );
  } else if (command === "windows-resources") {
    writeWindowsResources(directory);
  } else {
    throw new Error(
      `Usage: node ${fileURLToPath(import.meta.url)} name | windows-resources <directory>`,
    );
  }
}
