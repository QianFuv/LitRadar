/** Verify fixed packaged helpers and narrowly bounded read-only publisher discovery probes. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import { command } from "../primitives/run.mjs";
import { digest } from "../oracle.mjs";

/** Run one fresh CFP helper/public-source proof, retaining access failures as limitations. */
export async function runCfpLive() {
  const report = {
    phase: "cfp-live",
    checkedAt: new Date().toISOString(),
    result: "In Progress",
    commands: [],
  };
  const destination = "output/migration/execution/cfp-live-result.json";
  const save = () =>
    fs.writeFile(destination, JSON.stringify(report, null, 2) + "\n");
  const record = async (id, executable, args, timeout = 180000) => {
    report.pending = { id, executable, args };
    await save();
    const result = await command(id, executable, args, timeout);
    report.commands.push(result);
    delete report.pending;
    await save();
    return result;
  };
  const container = `litradar-cfp-${crypto.randomBytes(5).toString("hex")}`;
  let created = false;
  try {
    const image = await record("cfp-helper-image", "docker", [
      "image",
      "inspect",
      "litradar:migration-primitives-base",
      "--format",
      "{{.Id}}",
    ]);
    report.image = (await fs.readFile(image.log, "utf8")).trim();
    assert.equal(
      report.image,
      "sha256:f25c66d32a1bdc4e4a8c9d81e492533938fdff3d0bf5319a5311734a7735bb23",
    );
    await record("cfp-helper-probe-build", "wsl", [
      "-d",
      "Ubuntu",
      "--exec",
      "env",
      "GOWORK=off",
      "GOTOOLCHAIN=local",
      "GOENV=off",
      "GOFLAGS=",
      "CGO_ENABLED=1",
      "GOMODCACHE=/mnt/c/Users/57676/go/pkg/mod",
      "GOCACHE=/home/qianfuv/.cache/litradar-migration/go-cache",
      "/home/qianfuv/.cache/litradar-migration/go1.27.1/go/bin/go",
      "test",
      "-c",
      "-o",
      "output/migration/execution/cfp-packaged-proof",
      "./internal/cfp",
    ]);
    const binary = path.resolve(
      "output/migration/execution/cfp-packaged-proof",
    );
    report.probeSha256 = digest(await fs.readFile(binary));
    await record("cfp-helper-create", "docker", [
      "create",
      "--name",
      container,
      "--network",
      "none",
      "--read-only",
      "--user",
      "10001:10001",
      "--cap-drop",
      "ALL",
      "--security-opt",
      "no-new-privileges:true",
      "--tmpfs",
      "/tmp:rw,noexec,nosuid,nodev,size=64m",
      "--env",
      "LITRADAR_CFP_PACKAGED=1",
      "--mount",
      `type=bind,source=${binary},target=/usr/local/bin/cfp-proof,readonly`,
      "--entrypoint",
      "/usr/local/bin/cfp-proof",
      report.image,
      "-test.run=^TestRealPackagedHelpers$",
      "-test.v",
      "-test.timeout=100s",
    ]);
    created = true;
    const inspection = await record("cfp-helper-inspect", "docker", [
      "inspect",
      container,
    ]);
    const [settings] = JSON.parse(await fs.readFile(inspection.log, "utf8"));
    assert.equal(settings.HostConfig.ReadonlyRootfs, true);
    assert.equal(settings.Config.User, "10001:10001");
    assert.equal(settings.HostConfig.NetworkMode, "none");
    assert.deepEqual(settings.HostConfig.CapDrop, ["ALL"]);
    assert(settings.HostConfig.SecurityOpt.includes("no-new-privileges:true"));
    assert.equal(settings.Mounts.length, 1);
    assert.equal(settings.Mounts[0].RW, false);
    const helpers = await record(
      "cfp-real-helpers",
      "docker",
      ["start", "--attach", container],
      120000,
    );
    assert.match(
      await fs.readFile(helpers.log, "utf8"),
      /--- PASS: TestRealPackagedHelpers/,
    );
    report.helpers = {
      result: "Passed",
      realObscura: true,
      realPoppler: true,
      privateNetworkDenied: true,
      fixtureOnlyPrivateOverride: true,
      hardening: true,
    };
    const live = await record(
      "cfp-public-probes",
      "go",
      ["run", "./tests/migration/cfp/live-probe"],
      90000,
    );
    report.probes = JSON.parse(await fs.readFile(live.log, "utf8"));
    assert.equal(report.probes.length, 3);
    assert.equal(new Set(report.probes.map((probe) => probe.adapter)).size, 3);
    report.publicCoverage = report.probes.every(
      (probe) => probe.status === "Parsed",
    )
      ? "Passed"
      : "Limited";
    report.limitations = [
      "Each automatic adapter receives one discovery read with a 20-second bound; detail pages and all 467 registrations are not live-covered.",
      "Blocked/failed publisher access is retained verbatim and never counted as a successful capture. Fresh live checks are required before cutover.",
      "Only the isolated synthetic JavaScript fixture enables a private-network override. The production Go adapter removes that override and is separately verified to fail closed.",
      "Helper navigation/subresource policy is distinct from application host/path rules; no same-origin subresource or separate helper/key filesystem isolation is claimed.",
      "The fixed helper base verifies T09 adapters; final application image acceptance remains T12.",
    ];
    report.result = "Completed";
    await save();
    return {
      result: report.result,
      helpers: "Passed",
      publicCoverage: report.publicCoverage,
      probes: report.probes.map(({ adapter, status, error, parseError }) => ({
        adapter,
        status,
        error,
        parseError,
      })),
    };
  } catch (error) {
    report.result = "Failed";
    report.error = String(error);
    await save();
    throw error;
  } finally {
    if (created)
      await record("cfp-helper-cleanup", "docker", [
        "rm",
        "--force",
        container,
      ]);
  }
}
