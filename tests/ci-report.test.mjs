/** Verify portable artifact provenance and truthful final summaries without external services. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { writeCiReport, ciSummary } from "../scripts/ci-report.mjs";

const METADATA = {
  sourceSha: "a".repeat(40),
  platforms: ["linux/amd64"],
  stage: "build-acceptance",
  status: "success",
  requireFiles: true,
};

/** Allocate a test-owned artifact root with automatic cleanup. */
async function fixture(context) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-ci-report-"));
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}

test("manifest records source, target, portable paths and streamed SHA256 of exact payloads", async (context) => {
  const root = await fixture(context);
  await fs.mkdir(path.join(root, "assets"));
  const bytes = Buffer.alloc(2 * 1024 * 1024, 42);
  await fs.writeFile(path.join(root, "assets/image.tar"), bytes);
  await fs.writeFile(
    path.join(root, "assets/.hidden"),
    "excluded by normal artifact upload",
  );
  const report = await writeCiReport(root, ["assets"], {
    ...METADATA,
    checks: {
      smoke: {
        outcome: "success",
        outputs: { duration_seconds: "12", private_value: "not-for-report" },
      },
    },
  });
  assert.equal(report.sourceSha, METADATA.sourceSha);
  assert.deepEqual(report.platforms, ["linux/amd64"]);
  assert.deepEqual(report.files, [
    {
      path: "assets/image.tar",
      bytes: bytes.length,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    },
  ]);
  assert.equal(report.checks[0].durationSeconds, 12);
  assert(!JSON.stringify(report).includes("not-for-report"));
  const repeated = await writeCiReport(root, ["."], METADATA);
  assert.deepEqual(
    repeated.files,
    report.files,
    "The manifest cannot checksum itself",
  );
  const hidden = await writeCiReport(root, ["assets"], {
    ...METADATA,
    platforms: ["independent"],
    includeHidden: true,
  });
  assert(hidden.files.some((file) => file.path === "assets/.hidden"));
});

test("failure artifact inventory includes selected logs only, not extracted distributions", async (context) => {
  const root = await fixture(context);
  await fs.writeFile(path.join(root, "server.log"), "failure evidence");
  await fs.mkdir(path.join(root, "extracted"));
  await fs.writeFile(path.join(root, "extracted/litradar.exe"), "not uploaded");
  const report = await writeCiReport(root, ["server.log"], {
    ...METADATA,
    status: "failure",
  });
  assert.deepEqual(
    report.files.map((file) => file.path),
    ["server.log"],
  );
});

test("a manifest alone never makes a successful build artifact complete", async (context) => {
  const root = await fixture(context);
  await fs.writeFile(path.join(root, "ci-manifest.json"), "{}");
  await assert.rejects(
    writeCiReport(root, ["."], METADATA),
    /missing requested payloads/,
  );
  await assert.rejects(
    writeCiReport(root, ["absent.zip"], METADATA),
    /missing requested payloads/,
  );
  const failed = await writeCiReport(root, ["absent.log"], {
    ...METADATA,
    status: "failure",
  });
  assert.deepEqual(failed.missing, ["absent.log"]);
  assert.equal(failed.status, "failure");
  assert.match(ciSummary(failed), /Status: failure/);
});

test("invalid source/platform and escaping or linked paths fail closed", async (context) => {
  const root = await fixture(context);
  for (const metadata of [
    { ...METADATA, sourceSha: "main" },
    { ...METADATA, platforms: ["linux/arm64"] },
  ]) {
    await assert.rejects(writeCiReport(root, [], metadata));
  }
  await assert.rejects(
    writeCiReport(root, ["../outside"], METADATA),
    /escaped artifact root/,
  );
  const real = path.join(root, "real");
  await fs.mkdir(real);
  await fs.writeFile(path.join(real, "payload"), "unchanged");
  await fs.symlink(
    real,
    path.join(root, "link"),
    process.platform === "win32" ? "junction" : "dir",
  );
  await assert.rejects(
    writeCiReport(root, ["link/payload"], METADATA),
    /symbolic links/,
  );
  await assert.rejects(
    writeCiReport(
      root,
      [],
      { ...METADATA, requireFiles: false },
      "link/ci-manifest.json",
    ),
    /symbolic links/,
  );
});

test("final summary reflects a later upload failure without rewriting the uploaded manifest", async (context) => {
  const root = await fixture(context);
  await fs.writeFile(path.join(root, "payload"), "tested");
  await writeCiReport(root, ["payload"], METADATA);
  const summary = path.join(root, "summary.md");
  const result = spawnSync(
    process.execPath,
    [path.resolve("scripts/ci-report.mjs"), "summary"],
    {
      encoding: "utf8",
      timeout: 10000,
      windowsHide: true,
      env: {
        ...process.env,
        CI_REPORT_ROOT: root,
        CI_SOURCE_SHA: METADATA.sourceSha,
        CI_PLATFORMS: "linux/amd64",
        CI_STAGE: "build-acceptance",
        CI_STATUS: "failure",
        CI_CHECKS: JSON.stringify({ upload: { outcome: "failure" } }),
        GITHUB_STEP_SUMMARY: summary,
      },
    },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(await fs.readFile(summary, "utf8"), /Status: failure/);
  assert.equal(
    JSON.parse(await fs.readFile(path.join(root, "ci-manifest.json"), "utf8"))
      .status,
    "success",
  );
});

test("workflow standardization preserves required check identities and artifact download roots", async () => {
  const names = ["backend", "frontend", "ci", "release", "test-diagnostics"];
  const workflows = Object.fromEntries(
    await Promise.all(
      names.map(async (name) => [
        name,
        await fs.readFile(`.github/workflows/${name}.yaml`, "utf8"),
      ]),
    ),
  );
  for (const text of Object.values(workflows)) {
    assert(!text.includes("\t"));
    let blockIndent = null;
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      const indent = line.match(/^ */)[0].length;
      if (blockIndent !== null && indent > blockIndent) continue;
      blockIndent = /:\s*[>|][-+]?\s*$/.test(line) ? indent : null;
      assert.equal(indent % 2, 0, line);
    }
    assert.match(text, /ci-report.mjs summary/);
  }
  assert.match(workflows.backend, /name: Go checks \(\$\{\{ matrix.os \}\}\)/);
  assert.match(
    workflows.frontend,
    /  frontend:\s+outputs:/,
    "Preserve the implicit frontend job name",
  );
  assert.match(workflows.ci, /name: Release rules/);
  assert.match(workflows.ci, /name: Backend/);
  assert.match(workflows.ci, /name: Frontend/);
  assert.match(workflows.ci, /name: Version release/);
  assert.match(
    workflows.ci,
    /name: Verify release contracts\s+id: verify_release_contracts/,
  );
  for (const name of [
    "Package and smoke Windows distribution",
    "Build Linux image",
    "Smoke image and package tested binaries",
    "Publish binary release",
    "Promote the highest published stable version",
    "Verify public release and asset membership",
  ]) {
    assert.match(workflows.release, new RegExp(`name: ${name}\\s+id: [a-z_]+`));
  }
  assert.match(workflows.release, /-File -Filter \*\.log/);
  assert.match(workflows.release, /ci-report.mjs manifest @logs/);
  assert.match(
    workflows.release,
    /name: linux-release\s+path: release-results/,
  );
  assert.match(
    workflows.release,
    /name: windows-release\s+path: release-results\/windows/,
  );
  for (const name of ["frontend", "release", "test-diagnostics"]) {
    for (const file of [
      "go.mod",
      "go.sum",
      "third_party/go-sdk/go.mod",
      "third_party/go-sdk/go.sum",
      "third_party/go-sqlite3/go.mod",
      "third_party/go-sqlite3/go.sum",
    ])
      assert(
        workflows[name].includes(file),
        `${name} cache must declare ${file}`,
      );
  }
});
