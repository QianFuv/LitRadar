/** Verify native helpers and synthetic fixed-hostname delivery inside isolated hardened containers. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";

const root = process.cwd();
const directory = path.join(root, "output/migration/primitives/certificates");
const suffix = crypto.randomBytes(5).toString("hex");
const network = `litradar-primitives-${suffix}`;
const containers = new Set();
const image = "litradar:migration-primitives";
const hardening = [
  "--read-only",
  "--user",
  "10001:10001",
  "--cap-drop",
  "ALL",
  "--security-opt",
  "no-new-privileges:true",
  "--tmpfs",
  "/tmp:rw,noexec,nosuid,nodev,size=64m",
];

/** Run a bounded local command and preserve diagnostic output on failures. */
function command(executable, args, allowFailure = false, timeout = 30000) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    timeout,
    maxBuffer: 4 * 1024 * 1024,
    env: { ...process.env, GOWORK: "off", GOTOOLCHAIN: "go1.27.1" },
  });
  assert.ifError(result.error);
  if (!allowFailure)
    assert.equal(
      result.status,
      0,
      `${executable} failed: ${result.stderr}\n${result.stdout}`,
    );
  return result;
}

/** Create and inspect a container before any fixture process starts. */
function create(name, extra, args) {
  command("docker", [
    "create",
    "--name",
    name,
    "--network",
    network,
    ...hardening,
    "--mount",
    `type=bind,source=${directory},target=/fixtures,readonly`,
    ...extra,
    image,
    ...args,
  ]);
  containers.add(name);
  const [container] = JSON.parse(command("docker", ["inspect", name]).stdout);
  assert.deepEqual(Object.keys(container.NetworkSettings.Networks), [network]);
  assert.equal(container.HostConfig.ReadonlyRootfs, true);
  assert.equal(container.Config.User, "10001:10001");
  assert.deepEqual(container.HostConfig.CapDrop, ["ALL"]);
  assert(container.HostConfig.SecurityOpt.includes("no-new-privileges:true"));
  assert.equal(container.HostConfig.Privileged, false);
  assert.equal(container.HostConfig.PublishAllPorts, false);
  assert.equal(Object.keys(container.HostConfig.PortBindings ?? {}).length, 0);
  assert.equal(container.Mounts.length, 1);
  assert.equal(container.Mounts[0].Destination, "/fixtures");
  assert.equal(container.Mounts[0].RW, false);
  assert(
    !container.Config.Env.some((entry) =>
      /^(?:https?|all)_proxy=/i.test(entry),
    ),
  );
  const [net] = JSON.parse(
    command("docker", ["network", "inspect", network]).stdout,
  );
  assert.equal(net.Internal, true);
  return container;
}

/** Start a finite probe and return its verified process exit status. */
function run(name, extra, mode, timeout = 30000) {
  create(name, extra, ["-mode", mode]);
  const result = command("docker", ["start", "--attach", name], true, timeout);
  const [state] = JSON.parse(command("docker", ["inspect", name]).stdout);
  assert.equal(state.State.Running, false);
  return { ...result, status: state.State.ExitCode };
}

try {
  command("go", [
    "run",
    "./cmd/litradar-fixture",
    "-mode",
    "certificate",
    "-directory",
    directory,
  ]);
  await fs.mkdir(path.join(directory, "empty"), { recursive: true });
  command("docker", ["network", "create", "--internal", network]);
  const helpers = run(`litradar-helpers-${suffix}`, [], "helpers", 110000);
  assert.equal(helpers.status, 0, helpers.stderr);
  const helperEvidence = JSON.parse(helpers.stdout.trim());
  const ledger = `litradar-ledger-${suffix}`;
  create(ledger, ["--network-alias", "www.pushplus.plus"], ["-mode", "ledger"]);
  command("docker", ["start", ledger]);
  const trust = (name) => [
    "--env",
    `SSL_CERT_FILE=/fixtures/${name}-ca.pem`,
    "--env",
    "SSL_CERT_DIR=/fixtures/empty",
  ];
  const positive = run(`litradar-positive-${suffix}`, trust("valid"), "probe");
  assert.equal(positive.status, 0, positive.stderr);
  const deliveryEvidence = JSON.parse(positive.stdout.trim());
  const wrongCa = run(
    `litradar-wrong-ca-${suffix}`,
    trust("wrong-ca"),
    "probe",
  );
  assert.notEqual(wrongCa.status, 0);
  assert.match(wrongCa.stderr, /certificate signed by unknown authority/);
  const ledgerLog = command("docker", ["logs", ledger]).stdout;
  assert.equal(
    ledgerLog.split("\n").filter((line) => line.includes("synthetic-delivery"))
      .length,
    1,
  );
  command("docker", ["rm", "--force", ledger]);
  containers.delete(ledger);
  const wrongSanLedger = `litradar-wrong-san-ledger-${suffix}`;
  create(
    wrongSanLedger,
    [
      "--network-alias",
      "www.pushplus.plus",
      "--env",
      "LITRADAR_FIXTURE_CERT=wrong-san",
    ],
    ["-mode", "ledger"],
  );
  command("docker", ["start", wrongSanLedger]);
  const wrongSan = run(
    `litradar-wrong-san-${suffix}`,
    trust("wrong-san"),
    "probe",
  );
  assert.notEqual(wrongSan.status, 0);
  assert.match(
    wrongSan.stderr,
    /certificate is valid for wrong.fixture.invalid, not www.pushplus.plus/,
  );
  const report = {
    checkedAt: new Date().toISOString(),
    image: command("docker", [
      "image",
      "inspect",
      "--format",
      "{{.Id}}",
      image,
    ]).stdout.trim(),
    networkInternal: true,
    helpers: helperEvidence,
    fixedHost: deliveryEvidence,
    wrongCaRejected: true,
    wrongSanRejected: true,
  };
  await fs.writeFile(
    path.join(root, "output/migration/execution/t02-container-result.json"),
    JSON.stringify(report, null, 2) + "\n",
  );
  console.log(JSON.stringify(report, null, 2));
} finally {
  for (const container of containers)
    command("docker", ["rm", "--force", container], true);
  command("docker", ["network", "rm", network], true);
}
