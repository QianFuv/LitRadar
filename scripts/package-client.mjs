/** Stage client release bytes under immutable, versioned names. */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { parseVersion } from "./release-version.mjs";

/** Return the single supported artifact for each client platform. */
export function clientAssetName(version, platform) {
  parseVersion(version);
  const names = {
    web: `litradar-client-web_${version}_linux_amd64`,
    windows: `litradar-client_${version}_windows_amd64_setup.exe`,
    android: `litradar-client_${version}_android_arm64.apk`,
  };
  assert(Object.hasOwn(names, platform), "Unsupported client platform");
  return names[platform];
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
    windows: "litradar-client-windows-amd64-setup.exe",
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
  stageClient(process.argv[2], parseVersion(process.argv[3]));
}
