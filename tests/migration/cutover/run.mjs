/** Rehearse final-image delivery against copied historical state and isolated synthetic effects. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { BASELINE, loadOracle, digest } from "../oracle.mjs";
import { runVolumes } from "./volumes.mjs";
import { runManual } from "./manual.mjs";
import { cleanup } from "./cleanup.mjs";

const IMAGE = "litradar:go-test-amd64";
const HOST_FIXTURE = path.resolve(
  "output/migration/execution/cutover-fixture.exe",
);
const LINUX_FIXTURE = path.resolve(
  "output/migration/execution/cutover-fixture",
);

/** Preserve large integer identities as decimal strings in evidence rather than rounding them. */
function exactJson(text) {
  return JSON.parse(text, (key, value, context) =>
    typeof value === "number" &&
    !Number.isSafeInteger(value) &&
    /^-?\d+$/.test(context.source)
      ? context.source
      : value,
  );
}

/** Execute a bounded child and retain its exact diagnostics without shell interpolation. */
function command(executable, args, allowFailure = false, input) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 16 * 1024 * 1024,
    input,
  });
  assert.ifError(result.error);
  if (!allowFailure)
    assert.equal(
      result.status,
      0,
      `${executable} ${args.join(" ")}\n${result.stderr}\n${result.stdout}`,
    );
  return {
    status: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

/** Capture every regular copied file so sidecars and external evidence cannot disappear silently. */
async function inventory(directory, prefix = "") {
  const entries = [];
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const name = path.join(directory, entry.name),
      relative = `${prefix}${entry.name}`;
    assert(!entry.isSymbolicLink(), `Unexpected fixture link: ${name}`);
    if (entry.isDirectory())
      entries.push(...(await inventory(name, `${relative}/`)));
    else
      entries.push({ path: relative, sha256: digest(await fs.readFile(name)) });
  }
  return entries.sort((first, second) => first.path.localeCompare(second.path));
}

/** Bind phase receipts to all harness inputs and unchanged final application source identities. */
async function inputIdentity() {
  const paths = [
    "go.mod",
    "go.sum",
    "tests/migration/run.mjs",
    "tests/migration/oracle.mjs",
    "tests/data/migration/oracle-win32-x64.json",
  ];
  for (const directory of [
    "internal",
    "cmd",
    "tests/migration",
    "tests/data/migration",
  ]) {
    paths.push(
      ...(await inventory(directory))
        .filter(
          (item) =>
            !["internal", "cmd"].includes(directory) ||
            !item.path.endsWith("_test.go"),
        )
        .map((item) => `${directory}/${item.path}`),
    );
  }
  const pinned = JSON.parse(
    await fs.readFile("tests/migration/cutover/inputs.json", "utf8"),
  );
  paths.push(...pinned.files.map((item) => item.path));
  return Promise.all(
    [...new Set(paths)].sort().map(async (filename) => ({
      path: filename,
      sha256: digest(await fs.readFile(filename)),
    })),
  );
}

/** Build only the auxiliary test tool; the application always comes from the unchanged final image. */
function buildFixture() {
  command("go", [
    "build",
    "-tags",
    "sqlite_fts5,sqlite_dbstat",
    "-o",
    HOST_FIXTURE,
    "./tests/migration/cutover/fixture",
  ]);
  const result = spawnSync(
    "go",
    ["build", "-o", LINUX_FIXTURE, "./tests/migration/cutover/ledger"],
    {
      encoding: "utf8",
      windowsHide: true,
      timeout: 120000,
      env: { ...process.env, GOOS: "linux", GOARCH: "amd64", CGO_ENABLED: "0" },
    },
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
}

/** Assert hardening, exact image and network membership immediately before each application start. */
function inspect(name, network, image, members) {
  const [container] = JSON.parse(command("docker", ["inspect", name]).stdout);
  assert.equal(container.Image, image);
  assert.deepEqual(Object.keys(container.NetworkSettings.Networks), [network]);
  assert.equal(container.HostConfig.ReadonlyRootfs, true);
  assert.equal(container.Config.User, "10001:10001");
  assert.deepEqual(container.HostConfig.CapDrop, ["ALL"]);
  assert(container.HostConfig.SecurityOpt.includes("no-new-privileges:true"));
  assert.equal(container.HostConfig.Privileged, false);
  assert.equal(container.HostConfig.PublishAllPorts, false);
  assert.deepEqual(container.HostConfig.PortBindings ?? {}, {});
  assert(
    !container.Config.Env.some((value) =>
      /^(?:http|https|all)_proxy=/i.test(value),
    ),
  );
  assert(
    !container.Mounts.some((mount) =>
      /docker\.sock|\/proc|\/sys/.test(mount.Destination),
    ),
  );
  const [net] = JSON.parse(
    command("docker", ["network", "inspect", network]).stdout,
  );
  assert.equal(net.Internal, true);
  assert.equal(
    net.Options["com.docker.network.bridge.gateway_mode_ipv4"],
    "isolated",
  );
  for (const member of Object.values(net.Containers ?? {}))
    assert(members.has(member.Name));
  return { container, network: net };
}

/** Run the approved positive/response-loss/restart/TLS matrix with fresh per-case copied volumes. */
export async function runCutover(phase = "cutover") {
  assert(["cutover", "rollback"].includes(phase));
  const output = path.resolve("output/migration", phase);
  assert.equal(
    process.platform,
    "win32",
    "The retained Rust application oracle is Windows x64",
  );
  const oracle = await loadOracle(BASELINE);
  await fs.mkdir(output, { recursive: true });
  const inputs = await inputIdentity();
  buildFixture();
  const runId = crypto.randomBytes(6).toString("hex");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-cutover-"));
  const network = `litradar-cutover-${runId}`;
  const containers = new Set();
  const image = command("docker", [
    "image",
    "inspect",
    "--format",
    "{{.Id}}",
    IMAGE,
  ]).stdout.trim();
  const accepted = JSON.parse(
    await fs.readFile("tests/migration/cutover/inputs.json", "utf8"),
  );
  assert.equal(
    image,
    accepted.applicationImage,
    "Application image differs from the verified packaging receipt",
  );
  const evidence = {
    runId,
    root,
    image,
    inputs,
    toolchain: command("go", ["version"]).stdout.trim(),
    host: {
      platform: process.platform,
      architecture: process.arch,
      node: process.version,
    },
    status: "Running",
    startedAt: new Date().toISOString(),
    cases: [],
  };
  const hardening = [
    "--read-only",
    "--user",
    "10001:10001",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges:true",
    "--tmpfs",
    "/tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777",
    "--label",
    `litradar.cutover=${runId}`,
  ];
  const certificates = path.join(root, "certificates");
  const seed = path.join(root, "seed");
  /** Create a task-owned container without starting an application process. */
  function create(name, extra, args, isFixture = false) {
    command("docker", [
      "create",
      "--name",
      name,
      "--network",
      network,
      ...hardening,
      ...extra,
      ...(isFixture
        ? [
            "--mount",
            `type=bind,source=${LINUX_FIXTURE},target=/tools/fixture,readonly`,
            "--entrypoint",
            "/tools/fixture",
          ]
        : []),
      image,
      ...args,
    ]);
    containers.add(name);
    return inspect(name, network, image, containers);
  }
  /** Remove only a container owned by this invocation. */
  function remove(name) {
    command("docker", ["rm", "--force", name]);
    containers.delete(name);
  }
  try {
    await fs.mkdir(seed);
    await fs.writeFile(
      path.join(seed, ".litradar-e2e-root"),
      "litradar-full-stack-e2e-v1\n",
    );
    command(path.join(oracle.directory, "fullStackFixture.exe"), [
      "--project-root",
      seed,
    ]);
    await fs.writeFile(path.join(seed, "secret.key"), crypto.randomBytes(32));
    command(HOST_FIXTURE, ["prepare", seed]);
    const manifest = exactJson(
      await fs.readFile(
        path.join(seed, "data/push_state/full-stack.changes.json"),
        "utf8",
      ),
    );
    const articleId = String(manifest.notifiable_article_ids[0]);
    evidence.seed = await inventory(seed);
    if (phase === "rollback") {
      evidence.volumes = await runVolumes(
        {
          root,
          seed,
          image,
          oracle,
          fixture: HOST_FIXTURE,
          command,
          inventory,
          runId,
        },
        phase,
      );
      assert.deepEqual(
        await inputIdentity(),
        inputs,
        "Phase inputs changed during execution",
      );
      evidence.status = "Passed";
      return evidence;
    }
    command(HOST_FIXTURE, ["certificates", certificates]);
    await fs.mkdir(path.join(certificates, "empty"));
    const aiCa = await fs.readFile(path.join(certificates, "ai/valid-ca.pem"));
    for (const identity of ["valid", "wrong-ca", "wrong-san"]) {
      await fs.writeFile(
        path.join(certificates, `${identity}-trust.pem`),
        Buffer.concat([
          aiCa,
          await fs.readFile(path.join(certificates, `push/${identity}-ca.pem`)),
        ]),
      );
    }
    command("docker", [
      "network",
      "create",
      "--internal",
      "--subnet",
      "11.253.253.0/28",
      "--opt",
      "com.docker.network.bridge.gateway_mode_ipv4=isolated",
      network,
    ]);
    for (const scenario of [
      "positive",
      "response-lost",
      "commit-failure",
      "sending-kill",
      "wrong-ca",
      "wrong-san",
    ]) {
      const directory = path.join(root, scenario);
      await fs.cp(seed, directory, { recursive: true, errorOnExist: true });
      const keyPath = path.join(root, `${scenario}.key`);
      await fs.rename(path.join(directory, "secret.key"), keyPath);
      const item = { scenario, before: await inventory(directory), runs: [] };
      evidence.cases.push(item);
      const databaseSql = (sql) =>
        command(
          HOST_FIXTURE,
          ["database", directory],
          false,
          JSON.stringify({ path: "data/auth.sqlite", sql }),
        );
      if (scenario === "commit-failure")
        databaseSql(
          "CREATE TRIGGER reject_success BEFORE UPDATE OF status ON delivery_run_items WHEN NEW.status='succeeded' AND NEW.item_kind='subscriber' BEGIN SELECT RAISE(ABORT,'synthetic finalizer failure'); END;",
        );
      const ledger = `litradar-ledger-${runId}-${scenario}`;
      create(
        ledger,
        [
          "--network-alias",
          "www.pushplus.plus",
          "--ip",
          "11.253.253.2",
          "--mount",
          `type=bind,source=${certificates},target=/certificates,readonly`,
          "--env",
          `FIXTURE_ARTICLE_ID=${articleId}`,
          "--env",
          `FIXTURE_RESPONSE=${scenario === "response-lost" ? "lost" : scenario === "sending-kill" ? "hold" : "success"}`,
          "--env",
          `FIXTURE_CERT=${scenario === "wrong-san" ? "wrong-san" : "valid"}`,
        ],
        ["ledger", "/certificates"],
        true,
      );
      command("docker", ["start", ledger]);
      const deadline = Date.now() + 20000;
      while (true) {
        const log = command("docker", ["logs", ledger]).stdout;
        if (log.includes('"ledger-ready"')) break;
        assert(Date.now() < deadline, `Ledger not ready: ${log}`);
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
      for (
        let attempt = 0;
        attempt <
        (["response-lost", "commit-failure", "sending-kill"].includes(scenario)
          ? 2
          : 1);
        attempt++
      ) {
        const name = `litradar-app-${runId}-${scenario}-${attempt}`;
        const trust = ["wrong-ca", "wrong-san"].includes(scenario)
          ? scenario
          : "valid";
        const inspection = create(
          name,
          [
            "--mount",
            `type=bind,source=${directory},target=/fixture`,
            "--mount",
            `type=bind,source=${keyPath},target=/keys/secret.key,readonly`,
            "--mount",
            `type=bind,source=${certificates},target=/certificates,readonly`,
            "--env",
            `SSL_CERT_FILE=/certificates/${trust}-trust.pem`,
            "--env",
            "SSL_CERT_DIR=/certificates/empty",
          ],
          [
            "notify",
            "--project-root",
            "/fixture",
            "--secret-key-file",
            "/keys/secret.key",
            "--db",
            "full-stack.sqlite",
            "--changes-file",
            "/fixture/data/push_state/full-stack.changes.json",
            "--no-dry-run",
            "--timeout",
            scenario === "sending-kill" ? "60" : "10",
            "--retries",
            "1",
            "--max-candidates",
            "1",
          ],
        );
        let result;
        if (scenario === "sending-kill" && attempt === 0) {
          command("docker", ["start", name]);
          const barrierDeadline = Date.now() + 20000;
          while (
            !command("docker", ["logs", ledger]).stdout.includes(
              '"synthetic-delivery"',
            )
          ) {
            assert(
              Date.now() < barrierDeadline,
              "Sender did not reach receiver barrier",
            );
            await new Promise((resolve) => setTimeout(resolve, 100));
          }
          command("docker", ["kill", "--signal", "KILL", name]);
          command("docker", ["wait", name]);
          result = command("docker", ["logs", name]);
        } else result = command("docker", ["start", "--attach", name], true);
        const [finished] = JSON.parse(
          command("docker", ["inspect", name]).stdout,
        );
        assert.equal(finished.State.Running, false);
        assert.deepEqual(finished.Config.Entrypoint, ["litradar"]);
        const interrupted =
          attempt === 0 &&
          ["commit-failure", "sending-kill"].includes(scenario);
        const payload = interrupted ? null : exactJson(result.stdout.trim());
        const snapshot = JSON.parse(
          command(HOST_FIXTURE, [
            "snapshot",
            path.join(directory, "data/auth.sqlite"),
          ]).stdout,
        );
        item.runs.push({ inspection, result, payload, snapshot });
        if (interrupted) {
          assert.notEqual(finished.State.ExitCode, 0);
          if (scenario === "commit-failure") {
            assert(
              snapshot.delivery_dedupe.some((row) => row.status === "unknown"),
            );
            databaseSql("DROP TRIGGER reject_success");
          } else {
            assert.equal(finished.State.ExitCode, 137);
            assert(
              snapshot.delivery_run_items.some(
                (row) => row.status === "sending",
              ),
            );
            item.expirySimulation =
              "After confirmed process death, expire only lease timestamps in the disposable database; the original one-hour production lease is unchanged.";
            databaseSql(
              "UPDATE delivery_runs SET lease_expires_at=1 WHERE lease_expires_at IS NOT NULL; UPDATE delivery_run_items SET lease_expires_at=1 WHERE lease_expires_at IS NOT NULL; UPDATE delivery_leases SET expires_at=1 WHERE expires_at IS NOT NULL;",
            );
          }
          remove(name);
          continue;
        }
        assert.equal(payload.mode, "execute");
        if (attempt === 0) {
          assert(
            payload.databases[0].candidate_article_ids
              .map(String)
              .includes(articleId),
          );
          assert(
            payload.databases[0].subscribers[0].selected_article_ids
              .map(String)
              .includes(articleId),
          );
          assert.equal(
            payload.databases[0].subscribers[0].would_send_pushplus,
            true,
          );
        } else if (scenario !== "sending-kill") {
          assert.deepEqual(
            snapshot.delivery_dedupe,
            item.runs[0].snapshot.delivery_dedupe,
          );
          assert.deepEqual(
            snapshot.delivery_run_items,
            item.runs[0].snapshot.delivery_run_items,
          );
        }
        assert.equal(
          payload.status,
          scenario === "positive" ? "completed" : "unknown",
        );
        assert.equal(finished.State.ExitCode === 0, scenario === "positive");
        assert(
          snapshot.delivery_dedupe.some(
            (row) =>
              row.status ===
              (scenario === "positive" ? "confirmed" : "unknown"),
          ),
        );
        if (scenario === "positive")
          assert(
            snapshot.delivery_dedupe.some(
              (row) => row.message_id === "synthetic-message-id",
            ),
          );
        remove(name);
      }
      const logs = command("docker", ["logs", ledger]);
      item.ledger = logs;
      assert(
        logs.stdout.includes('"synthetic-ai"'),
        "TLS negative control must reach AI selection first",
      );
      assert.equal(
        logs.stdout
          .split("\n")
          .filter((line) => line.includes('"synthetic-delivery"')).length,
        [
          "positive",
          "response-lost",
          "commit-failure",
          "sending-kill",
        ].includes(scenario)
          ? 1
          : 0,
      );
      item.after = await inventory(directory);
      remove(ledger);
    }
    evidence.manual = await runManual({
      root,
      seed,
      certificates,
      runId,
      articleId,
      fixture: HOST_FIXTURE,
      create,
      remove,
      command,
      inventory,
      inspect: (name) => inspect(name, network, image, containers),
    });
    evidence.volumes = await runVolumes({
      root,
      seed,
      image,
      oracle,
      fixture: HOST_FIXTURE,
      command,
      inventory,
      runId,
    });
    assert.deepEqual(
      await inputIdentity(),
      inputs,
      "Phase inputs changed during execution",
    );
    evidence.status = "Passed";
    return evidence;
  } catch (error) {
    evidence.status = "Failed";
    evidence.error = error.stack;
    throw error;
  } finally {
    evidence.cleanup = cleanup(
      command,
      containers,
      phase === "cutover" ? network : undefined,
    );
    const failedCleanup = evidence.cleanup.some((item) => !item.success);
    if (failedCleanup) evidence.status = "Failed";
    evidence.finishedAt = new Date().toISOString();
    await fs.writeFile(
      path.join(output, `${runId}.json`),
      JSON.stringify(evidence, null, 2) + "\n",
    );
    await fs.writeFile(
      path.join(output, "result.json"),
      JSON.stringify(evidence, null, 2) + "\n",
    );
    if (failedCleanup)
      throw new Error("Cutover resource cleanup failed; see the phase receipt");
  }
}
