/** Install hash-pinned release scanners into the task-local tool directory. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";

assert(["win32", "linux"].includes(process.platform) && process.arch === "x64");
const manifest = JSON.parse(
  await fs.readFile(new URL("security-tools.json", import.meta.url), "utf8"),
);
const destination = path.resolve("output/security/tools");
await fs.mkdir(destination, { recursive: true });
const extension = process.platform === "win32" ? ".exe" : "";
for (const tool of manifest.tools) {
  const asset = tool.assets[process.platform];
  const archive = path.join(
    destination,
    path.basename(new URL(asset.url).pathname),
  );
  let bytes;
  try {
    bytes = await fs.readFile(archive);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    const response = await fetch(asset.url, {
      signal: AbortSignal.timeout(180000),
    });
    assert(response.ok, `${tool.name}: HTTP ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      asset.sha256,
    );
    await fs.writeFile(archive, bytes);
  }
  assert.equal(createHash("sha256").update(bytes).digest("hex"), asset.sha256);
  const binary = path.join(destination, tool.name + extension);
  if (tool.name === "osv-scanner") await fs.copyFile(archive, binary);
  else {
    const listing = spawnSync("tar", ["-tf", archive], {
      encoding: "utf8",
      timeout: 30000,
      windowsHide: true,
    });
    assert.ifError(listing.error);
    assert.equal(listing.status, 0, listing.stderr);
    const matches = listing.stdout
      .trim()
      .split(/\r?\n/)
      .filter((entry) => path.posix.basename(entry) === tool.name + extension);
    assert.equal(matches.length, 1);
    assert(
      !matches[0].startsWith("/") && !matches[0].split("/").includes(".."),
    );
    const result = spawnSync(
      "tar",
      [
        "-xf",
        archive,
        "-C",
        destination,
        "--strip-components",
        String(matches[0].split("/").length - 1),
        matches[0],
      ],
      { encoding: "utf8", timeout: 30000, windowsHide: true },
    );
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
  }
  await fs.chmod(binary, 0o755);
  console.log(`Verified ${tool.name} ${tool.version}`);
}
const installation = spawnSync(
  "go",
  ["install", `golang.org/x/vuln/cmd/govulncheck@${manifest.govulncheck}`],
  {
    env: {
      ...process.env,
      GOBIN: destination,
      GOWORK: "off",
      GOTOOLCHAIN: "go1.27.1",
      GOENV: "off",
      GOFLAGS: "",
    },
    timeout: 300000,
    windowsHide: true,
    stdio: "inherit",
  },
);
assert.ifError(installation.error);
assert.equal(installation.status, 0);
