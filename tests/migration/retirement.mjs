/** Prove the retired source tree builds from a copied context with no old build outputs. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { digest } from "./oracle.mjs";
import { verifyFrozenEvidence } from "./frozen-evidence.mjs";

const RETIRED = [
  "crates",
  "Cargo.toml",
  "Cargo.lock",
  ".config/nextest.toml",
  "deny.toml",
  "osv-scanner.toml",
  "scripts/filter-codeql-sarif.mjs",
  "tests/filter-codeql-sarif.test.mjs",
];

/** Build the ordinary binaries and both release images without relying on an existing checkout artifact. */
export async function runRetirement() {
  const output = path.resolve("output/migration/retirement");
  await fs.mkdir(output, { recursive: true });
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-retirement-"));
  const report = {
    status: "Running",
    startedAt: new Date().toISOString(),
    root,
    retired: RETIRED,
    inputs: [],
    commands: [],
    images: {},
  };
  const save = () =>
    fs.writeFile(
      path.join(output, "result.json"),
      JSON.stringify(report, null, 2) + "\n",
    );
  /** Bound each managed command and retain full stdout/stderr in a separate diagnostic file. */
  async function command(id, executable, args, cwd = root, timeout = 300000) {
    report.pending = { id, executable, args, cwd, timeout };
    await save();
    const result = spawnSync(executable, args, {
      cwd,
      encoding: "utf8",
      windowsHide: true,
      timeout,
      maxBuffer: 32 * 1024 * 1024,
    });
    const log = path.join(output, `${id}.log`);
    await fs.writeFile(log, `${result.stdout ?? ""}\n${result.stderr ?? ""}`);
    report.commands.push({
      ...report.pending,
      log,
      exitCode: result.status,
      error: result.error?.message,
    });
    delete report.pending;
    await save();
    assert.ifError(result.error);
    assert.equal(result.status, 0, `${id} failed; see ${log}`);
    return result.stdout.trim();
  }
  try {
    for (const filename of RETIRED)
      await assert.rejects(fs.lstat(filename), { code: "ENOENT" });
    report.frozen = await verifyFrozenEvidence();
    const tracked = await command(
      "source-inventory",
      "git",
      ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
      process.cwd(),
    );
    const names = [...new Set(tracked.split("\0").filter(Boolean))].sort();
    for (const filename of names) {
      if (
        /^(?:\.git\/|target\/|output\/|data\/|node_modules\/|app\/(?:node_modules|out|\.next)\/|docs\/plan\/)/.test(
          filename,
        )
      )
        continue;
      let metadata;
      try {
        metadata = await fs.lstat(filename);
      } catch (error) {
        if (error.code === "ENOENT") continue;
        throw error;
      }
      assert(
        metadata.isFile() && !metadata.isSymbolicLink(),
        `Unexpected source input: ${filename}`,
      );
      assert(!filename.split("/").includes("..") && !path.isAbsolute(filename));
      const bytes = await fs.readFile(filename);
      const destination = path.join(root, filename);
      await fs.mkdir(path.dirname(destination), { recursive: true });
      await fs.writeFile(destination, bytes);
      report.inputs.push({ path: filename, sha256: digest(bytes) });
    }
    for (const filename of [
      ...RETIRED,
      ".git",
      "target",
      "output",
      "app/node_modules",
      "app/out",
    ])
      await assert.rejects(fs.lstat(path.join(root, filename)), {
        code: "ENOENT",
      });
    await save();
    await command("go-build", process.execPath, ["scripts/build-go.mjs"]);
    const executable = path.join(
      root,
      "target/go",
      process.platform === "win32" ? "litradar.exe" : "litradar",
    );
    assert.match(await command("public-help", executable, ["--help"]), /serve/);
    const document = JSON.parse(
      await command("openapi", executable, ["openapi"]),
    );
    assert(Object.keys(document.paths).length > 0);
    for (const architecture of ["amd64", "arm64"]) {
      const tag = `litradar:go-test-${architecture}`;
      await command(
        `image-${architecture}`,
        "docker",
        [
          "buildx",
          "build",
          "--platform",
          `linux/${architecture}`,
          "--load",
          "--provenance=false",
          "-t",
          tag,
          ".",
        ],
        root,
        900000,
      );
      report.images[architecture] = await command(
        `identity-${architecture}`,
        "docker",
        ["image", "inspect", "--format", "{{.Id}}", tag],
      );
    }
    for (const entry of report.inputs)
      assert.equal(
        digest(await fs.readFile(entry.path)),
        entry.sha256,
        `Source changed during clean build: ${entry.path}`,
      );
    report.status = "Passed";
    return report;
  } catch (error) {
    report.status = "Failed";
    report.error = String(error.stack ?? error);
    throw error;
  } finally {
    report.finishedAt = new Date().toISOString();
    await save();
  }
}
