/** Package the exact tested Linux image bytes as a relocatable native distribution. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { parseVersion } from "./release-version.mjs";

const [image, architecture, inputVersion] = process.argv.slice(2);
assert(image && architecture === "amd64");
const version = parseVersion(inputVersion);
const name = `litradar_${version}_linux_${architecture}`;
const parent = path.resolve("release-results/packages");
const directory = path.join(parent, name);
assert(!fs.existsSync(directory), "Package directory already exists");
fs.mkdirSync(directory, { recursive: true });
const container = `litradar-package-${randomUUID()}`;
/** Execute packaging tools without shell interpolation. */
function run(command, args) {
  return execFileSync(command, args, { encoding: "utf8", timeout: 300000 });
}
run("docker", ["create", "--name", container, image]);
try {
  for (const [source, destination] of [
    ["/usr/local/bin/litradar", "litradar"],
    ["/usr/local/bin/obscura", "obscura"],
    ["/usr/local/bin/obscura-worker", "obscura-worker"],
    ["/usr/share/litradar/meta", "assets/meta"],
    ["/usr/share/doc/litradar/third-party", "licenses"],
  ]) {
    const target = path.join(directory, destination);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    run("docker", ["cp", `${container}:${source}`, target]);
  }
} finally {
  run("docker", ["rm", container]);
}
for (const executable of ["litradar", "obscura", "obscura-worker"])
  fs.chmodSync(path.join(directory, executable), 0o755);
fs.copyFileSync("LICENSE", path.join(directory, "LICENSE"));
fs.copyFileSync(
  "docs/operations/releases.md",
  path.join(directory, "README.md"),
);
fs.writeFileSync(path.join(directory, "VERSION"), version + "\n");
fs.writeFileSync(
  path.join(directory, "run.sh"),
  `#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export LITRADAR_OBSCURA_PATH="\${LITRADAR_OBSCURA_PATH:-$root/obscura}"
cd "$root"
exec "$root/litradar" "$@"
`,
  { mode: 0o755 },
);
const assets = path.resolve("release-results/assets");
fs.mkdirSync(assets, { recursive: true });
const filename = `${name}.tar.gz`;
run("tar", ["-czf", path.join(assets, filename), "-C", parent, name]);
run("docker", [
  "run",
  "--rm",
  "--network",
  "none",
  "--tmpfs",
  "/usr/lib/litradar",
  "--tmpfs",
  "/usr/share/litradar/meta",
  "--mount",
  `type=bind,source=${assets},target=/release-assets,readonly`,
  "--mount",
  `type=bind,source=${path.resolve("scripts/smoke-release.sh")},target=/smoke-release.sh,readonly`,
  "--mount",
  `type=bind,source=${path.resolve("test-results/container-smoke/search-fixture.sqlite")},target=/smoke-fixture.sqlite,readonly`,
  "--env",
  "LITRADAR_OBSCURA_PATH=",
  "--entrypoint",
  "sh",
  image,
  "/smoke-release.sh",
  filename,
  name,
  version,
]);
const hash = createHash("sha256")
  .update(fs.readFileSync(path.join(assets, filename)))
  .digest("hex");
fs.appendFileSync(path.join(assets, "SHA256SUMS"), `${hash}  ${filename}\n`);
console.log(filename);
