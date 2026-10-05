/** Exercise copied-volume startup, historical formats, secret rotation and offline recovery. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import {
  runSchedulerVolumes,
  runCfpVolumes,
  rows,
} from "./persistent-jobs.mjs";
import { runFormats } from "./formats.mjs";
import { runIndexVolume } from "./index-flow.mjs";
import { runRecovery } from "./recovery.mjs";
import { cleanup } from "./cleanup.mjs";

/** Test public packaged commands on disposable roots and retain complete before/after inventories. */
export async function runVolumes(context, phase = "cutover") {
  const { root, seed, image, oracle, fixture, command, inventory, runId } =
    context;
  const result = { cases: [], image, status: "Running" };
  const containers = new Set();
  let sequence = 0;
  /** Permit SQLite to remove empty WAL coordination files while retaining every durable byte. */
  function durableFiles(entries) {
    const emptyWal =
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const empty = new Set(
      entries
        .filter(
          (item) => item.path.endsWith("-wal") && item.sha256 === emptyWal,
        )
        .map((item) => item.path.slice(0, -4)),
    );
    return entries.filter(
      (item) =>
        !["-wal", "-shm"].some(
          (suffix) =>
            item.path.endsWith(suffix) && empty.has(item.path.slice(0, -4)),
        ),
    );
  }
  /** Preserve storage classes and exact integer identities in an offline semantic snapshot. */
  function database(directory, relative = "data/auth.sqlite", sql = "") {
    const output = command(
      fixture,
      ["database", directory],
      false,
      JSON.stringify({ path: relative, sql }),
    );
    return sql ? undefined : JSON.parse(output.stdout);
  }
  /** Copy all state, not only database main files, before each destructive rehearsal. */
  async function copy(name, source = seed) {
    const directory = path.join(root, name);
    await fs.cp(source, directory, { recursive: true, errorOnExist: true });
    await fs.rm(path.join(directory, "secret.key"), { force: true });
    return directory;
  }
  /** Construct an unmodified final application with no network and a separately mounted key. */
  function create(directory, args, key = path.join(seed, "secret.key")) {
    const name = `litradar-volume-${runId}-${sequence++}`;
    const isService = args[0] === "serve";
    const rotation = args.includes("--new-key-file")
      ? [
          "--mount",
          `type=bind,source=${path.join(root, "rotation.key")},target=/keys/next,readonly`,
        ]
      : [];
    const external = args.includes("/external/auth.sqlite")
      ? [
          "--mount",
          `type=bind,source=${path.join(directory, "external-auth")},target=/external`,
        ]
      : [];
    command("docker", [
      "create",
      "--name",
      name,
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
      "/tmp:rw,nosuid,nodev,size=64m,mode=1777",
      "--mount",
      isService
        ? `type=bind,source=${path.join(directory, "data")},target=/app/data`
        : `type=bind,source=${directory},target=/fixture`,
      "--mount",
      `type=bind,source=${key},target=/keys/key,readonly`,
      ...rotation,
      ...external,
      image,
      ...args,
      "--project-root",
      isService ? "/app" : "/fixture",
    ]);
    containers.add(name);
    const [inspection] = JSON.parse(
      command("docker", ["inspect", name]).stdout,
    );
    assert.equal(inspection.Image, image);
    assert.deepEqual(inspection.Config.Entrypoint, ["litradar"]);
    assert.equal(inspection.HostConfig.NetworkMode, "none");
    assert.equal(inspection.HostConfig.ReadonlyRootfs, true);
    assert.equal(
      inspection.Mounts.find((mount) => mount.Destination === "/keys/key").RW,
      false,
    );
    if (rotation.length)
      assert.equal(
        inspection.Mounts.find((mount) => mount.Destination === "/keys/next")
          .RW,
        false,
      );
    assert(
      !inspection.Config.Env.some((value) =>
        /^(?:https?|all)_proxy=/i.test(value),
      ),
    );
    return name;
  }
  /** Run one finite command and prove the whole container stopped before observing its files. */
  function application(directory, args, allowFailure = false, key) {
    const name = create(directory, args, key);
    const output = command("docker", ["start", "--attach", name], true);
    const [inspection] = JSON.parse(
      command("docker", ["inspect", name]).stdout,
    );
    assert.equal(inspection.State.Running, false);
    output.status = inspection.State.ExitCode;
    command("docker", ["rm", name]);
    containers.delete(name);
    if (!allowFailure) assert.equal(output.status, 0, JSON.stringify(output));
    return output;
  }
  /** Exercise real HTTP admission, persisted sessions and FTS inside the networkless service container. */
  async function service(directory, afterReady) {
    const name = create(directory, [
      "serve",
      "--host",
      "127.0.0.1",
      "--port",
      "8000",
      "--secret-key-file",
      "/keys/key",
    ]);
    command("docker", ["start", name]);
    const deadline = Date.now() + 45000;
    while (true) {
      const ready = command(
        "docker",
        [
          "exec",
          name,
          "curl",
          "--fail",
          "--silent",
          "http://127.0.0.1:8000/health/ready",
        ],
        true,
      );
      if (ready.status === 0) break;
      const [state] = JSON.parse(command("docker", ["inspect", name]).stdout);
      assert(
        state.State.Running && Date.now() < deadline,
        command("docker", ["logs", name]).stderr,
      );
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    const login = command("docker", [
      "exec",
      name,
      "curl",
      "--fail",
      "--silent",
      "--cookie-jar",
      "/tmp/cookies",
      "--header",
      "Content-Type: application/json",
      "--data",
      JSON.stringify({
        username: "fullstack_admin",
        password: "FullStackAdmin!2026",
      }),
      "http://127.0.0.1:8000/api/auth/login",
    ]);
    const me = command("docker", [
      "exec",
      name,
      "curl",
      "--fail",
      "--silent",
      "--cookie",
      "/tmp/cookies",
      "http://127.0.0.1:8000/api/auth/me",
    ]);
    assert.match(me.stdout, /fullstack_admin/);
    const articles = command("docker", [
      "exec",
      name,
      "curl",
      "--fail",
      "--silent",
      "--cookie",
      "/tmp/cookies",
      "http://127.0.0.1:8000/api/articles?db=full-stack.sqlite&q=Evidence",
    ]);
    assert.match(
      articles.stdout,
      /Evidence Graphs for Living Literature Reviews/,
    );
    if (afterReady) await afterReady(name);
    command("docker", ["stop", "--time", "15", name]);
    const [stopped] = JSON.parse(command("docker", ["inspect", name]).stdout);
    assert.equal(stopped.State.Running, false);
    assert.equal(stopped.State.ExitCode, 0);
    command("docker", ["rm", name]);
    containers.delete(name);
    return { login, me, articles };
  }
  /** Run the frozen historical executable only on an offline disposable copy. */
  function historical(directory, args) {
    return command(path.join(oracle.directory, "application.exe"), [
      ...args,
      "--project-root",
      directory,
    ]);
  }
  try {
    if (phase === "rollback") {
      result.cases = await runRecovery({
        copy,
        database,
        application,
        historical,
        service,
        inventory,
        seed,
      });
      result.status = "Passed";
      return result;
    }
    const current = await copy("whole-volume-current");
    const first = {
      name: "rust-to-go-startup-and-current-state-rollback",
      before: await inventory(current),
      authBefore: database(current),
    };
    first.http = await service(current);
    first.goVerify = application(current, [
      "admin",
      "secrets",
      "verify",
      "--secret-key-file",
      "/keys/key",
    ]);
    const rollback = await copy("current-state-rust", current);
    first.beforeRust = database(rollback);
    first.rustVerify = historical(rollback, [
      "admin",
      "secrets",
      "verify",
      "--secret-key-file",
      path.join(seed, "secret.key"),
    ]);
    first.rustScheduler = historical(rollback, [
      "scheduler",
      "validate",
      "--secret-key-file",
      path.join(seed, "secret.key"),
    ]);
    assert.deepEqual(
      database(rollback),
      first.beforeRust,
      "Historical current-state verification changed logical auth state",
    );
    first.returnToGo = await service(rollback);
    first.after = await inventory(rollback);
    assert.deepEqual(database(rollback).users, first.authBefore.users);
    result.cases.push(first);

    const external = await copy("external-auth-path");
    await fs.mkdir(path.join(external, "external-auth"));
    for (const name of (await fs.readdir(path.join(external, "data"))).filter(
      (name) => name === "auth.sqlite" || name.startsWith("auth.sqlite-"),
    )) {
      await fs.rename(
        path.join(external, "data", name),
        path.join(external, "external-auth", name),
      );
    }
    const externalBefore = database(external, "external-auth/auth.sqlite");
    const externalGo = application(external, [
      "admin",
      "secrets",
      "verify",
      "--auth-db",
      "/external/auth.sqlite",
      "--secret-key-file",
      "/keys/key",
    ]);
    const externalScheduler = application(external, [
      "scheduler",
      "validate",
      "--auth-db",
      "/external/auth.sqlite",
      "--secret-key-file",
      "/keys/key",
    ]);
    const externalRust = historical(external, [
      "admin",
      "secrets",
      "verify",
      "--auth-db",
      path.join(external, "external-auth/auth.sqlite"),
      "--secret-key-file",
      path.join(seed, "secret.key"),
    ]);
    assert.deepEqual(
      database(external, "external-auth/auth.sqlite").users,
      externalBefore.users,
    );
    assert(
      !(await fs.readdir(path.join(external, "data"))).includes("auth.sqlite"),
      "Explicit external auth path fell back to the project default",
    );
    result.cases.push({
      name: "external-auth-path-and-sidecars",
      before: externalBefore,
      go: externalGo,
      scheduler: externalScheduler,
      boundary:
        "Explicit --auth-db is supported by maintenance and worker commands. Both original and Go serve reject this option; deployment directory mounts remain the service path mechanism.",
      rust: externalRust,
      after: await inventory(external),
    });

    const rotate = await copy("secret-rotation");
    const nextKey = path.join(root, "rotation.key");
    await fs.writeFile(nextKey, crypto.randomBytes(32));
    const secretCase = {
      name: "go-secret-rotation-rust-read",
      before: database(rotate),
    };
    secretCase.rotate = application(rotate, [
      "admin",
      "secrets",
      "rotate",
      "--old-key-file",
      "/keys/key",
      "--new-key-file",
      "/keys/next",
    ]);
    secretCase.oldKey = application(
      rotate,
      ["admin", "secrets", "verify", "--secret-key-file", "/keys/key"],
      true,
    );
    assert.notEqual(secretCase.oldKey.status, 0);
    secretCase.rust = historical(rotate, [
      "admin",
      "secrets",
      "verify",
      "--secret-key-file",
      nextKey,
    ]);
    secretCase.newKey = application(
      rotate,
      ["admin", "secrets", "verify", "--secret-key-file", "/keys/key"],
      false,
      nextKey,
    );
    assert.deepEqual(database(rotate).users, secretCase.before.users);
    result.cases.push(secretCase);

    for (const marker of [
      ".litradar-index-maintenance.json",
      ".litradar-index-staging",
      ".litradar-index-rollback",
    ]) {
      const directory = await copy(`interrupted-${marker}`);
      await fs.writeFile(
        path.join(directory, "data", marker),
        "interrupted synthetic maintenance\n",
      );
      const before = await inventory(directory);
      const outcome = application(
        directory,
        ["admin", "index", "optimize-storage", "--confirm-index-maintenance"],
        true,
      );
      assert.notEqual(outcome.status, 0);
      assert.equal(JSON.parse(outcome.stdout).error.code, "interrupted_state");
      assert.deepEqual(await inventory(directory), before);
      result.cases.push({ name: marker, before, outcome, unchanged: true });
    }

    const jobsContext = {
      root,
      copy,
      database,
      service,
      application,
      command,
      inventory,
      fixture,
    };
    result.scheduler = await runSchedulerVolumes(jobsContext);
    result.cfp = await runCfpVolumes(jobsContext);
    result.formats = await runFormats(jobsContext);
    result.index = await runIndexVolume(jobsContext);
    const backupRoot = await copy("backup-source", result.index.directory);
    const backups = {
      name: "final-image-backup-and-mixed-epoch",
      before: await inventory(backupRoot),
    };
    backups.create = application(backupRoot, [
      "admin",
      "backup",
      "create",
      "--output",
      "/fixture/backup",
      "--include-indexes",
      "--include-push-state",
    ]);
    backups.verify = application(backupRoot, [
      "admin",
      "backup",
      "verify",
      "--backup",
      "/fixture/backup",
    ]);
    backups.rustVerify = historical(backupRoot, [
      "admin",
      "backup",
      "verify",
      "--backup",
      path.join(backupRoot, "backup"),
    ]);
    const backupInventory = await inventory(path.join(backupRoot, "backup"));
    assert(!backupInventory.some((item) => item.path.endsWith(".key")));
    const restore = await copy("backup-restore", backupRoot);
    for (const name of (
      await fs.readdir(path.join(root, "response-lost/data"))
    ).filter(
      (name) => name === "auth.sqlite" || name.startsWith("auth.sqlite-"),
    )) {
      await fs.copyFile(
        path.join(root, "response-lost/data", name),
        path.join(restore, "data", name),
      );
    }
    const newerWork = JSON.parse(
      command(fixture, ["index-notify-seed", restore]).stdout,
    );
    database(
      restore,
      "data/index/fixture.sqlite",
      "UPDATE articles SET title='Content after the old snapshot' WHERE title='Historical rollback added article'",
    );
    backups.newerDelivery = database(restore).delivery_dedupe;
    assert(
      backups.newerDelivery.rows.some((value) => value.includes('"unknown"')),
    );
    database(
      restore,
      "data/auth.sqlite",
      "UPDATE users SET username='changed_after_snapshot' WHERE id=1",
    );
    await fs.mkdir(path.join(restore, "data/index-control"), {
      recursive: true,
    });
    const batchCorpus = JSON.parse(
      await fs.readFile("tests/migration/index/batch-vectors.json", "utf8"),
    );
    const batchInput = structuredClone(
      batchCorpus.observations.find(
        (item) => item.input.name === "notification-unknown",
      ).input,
    );
    batchInput.path = path.join(
      restore,
      "data/index-control/index-batches.sqlite",
    );
    batchInput.operations = [
      ...batchInput.operations.slice(0, 8),
      { op: "release" },
    ];
    batchInput.operations[0].request = newerWork.oracleRequest;
    batchInput.operations[2].value.journals = 2;
    backups.newerBatch = command(
      path.resolve("output/migration/execution/index-oracle/identity.exe"),
      [],
      false,
      JSON.stringify(batchInput) + "\n",
    );
    assert(
      JSON.parse(backups.newerBatch.stdout).expected.operations.every(
        (operation) => !operation.error,
      ),
      backups.newerBatch.stdout,
    );
    const batchBefore = database(
      restore,
      "data/index-control/index-batches.sqlite",
    );
    const unknown = rows(batchBefore, "index_batch_catalogs").find(
      (row) => row.batch_id === newerWork.batchId,
    );
    assert.equal(unknown.catalog_name, "fixture");
    assert.equal(unknown.notify_status, "unknown");
    assert.equal(unknown.notify_attempt_id, "attempt-1");
    assert.equal(unknown.phase, "notifying");
    assert.equal(batchBefore.index_batch_lease.rows.length, 0);
    const newerEpoch = await copy("disaster-newer-volume", restore);
    const quarantinedAuth = database(newerEpoch);
    backups.quarantinedEpoch = {
      path: newerEpoch,
      inventory: await inventory(newerEpoch),
      auth: quarantinedAuth,
    };
    const controlBefore = await inventory(
      path.join(restore, "data/index-control"),
    );
    const workBefore = await inventory(path.join(restore, "data/index-work"));
    assert(workBefore.length >= 4, "Both prepared owned worksets must exist");
    const checkpoints = rows(
      database(restore, "data/index-control/fixture.sqlite"),
      "provider_run_checkpoints",
    );
    assert.deepEqual(checkpoints.map((row) => row.catalog_id).sort(), [
      "fixture-0",
      "fixture-1",
    ]);
    const heartbeatRows = database(restore).scheduler_workers.rows.map(
      (value) => JSON.parse(value),
    );
    const latestHeartbeat = Math.max(
      0,
      ...heartbeatRows.map((row) => Number(row[2][1])),
    );
    const remaining = Math.max(0, (latestHeartbeat + 91) * 1000 - Date.now());
    assert(
      remaining <= 91000,
      "Unexpected future heartbeat in synthetic snapshot",
    );
    if (remaining)
      await new Promise((resolve) => setTimeout(resolve, remaining));
    backups.restore = application(restore, [
      "admin",
      "backup",
      "restore",
      "--backup",
      "/fixture/backup",
      "--confirm-restore",
    ]);
    assert.deepEqual(database(restore).users, database(backupRoot).users);
    assert.equal(database(restore).delivery_dedupe.rows.length, 0);
    assert.deepEqual(
      await inventory(newerEpoch),
      backups.quarantinedEpoch.inventory,
    );
    assert.deepEqual(
      database(newerEpoch).delivery_dedupe,
      backups.newerDelivery,
    );
    assert.deepEqual(
      durableFiles(await inventory(path.join(restore, "data/index-control"))),
      durableFiles(controlBefore),
      "Backup restore must not silently discard newer external-effect evidence",
    );
    assert.deepEqual(
      await inventory(path.join(restore, "data/index-work")),
      workBefore,
    );
    assert.deepEqual(
      database(restore, "data/index-control/index-batches.sqlite"),
      batchBefore,
    );
    assert.deepEqual(
      database(restore, "data/index/fixture.sqlite").articles,
      database(backupRoot, "data/index/fixture.sqlite").articles,
    );
    backups.controlPreserved = controlBefore;
    backups.sqliteHousekeeping =
      "Read/write restore preflight may remove a zero-byte WAL and its SHM; main database hashes and complete logical records must remain identical. Nonempty WALs are never excluded.";
    backups.worksetsPreserved = workBefore;
    backups.after = await inventory(restore);
    backups.recoveryDisposition =
      "Restored groups and newer control evidence are a mixed epoch. Admission remains stopped; preserve both snapshots and reconcile Unknown before restarting.";
    result.cases.push(backups);
    const workset = workBefore.find((item) => item.path.endsWith(".sqlite"));
    assert(
      workset,
      "A real owned workset is required for missing-ledger recovery",
    );
    for (const [ordinal, missing] of [
      "data/index-control/index-batches.sqlite",
      "data/index-control/fixture.sqlite",
      `data/index-work/${workset.path}`,
    ].entries()) {
      const directory = await copy(`missing-ledger-${ordinal}`, newerEpoch);
      const target = path.resolve(directory, missing);
      assert(target.startsWith(`${path.resolve(directory)}${path.sep}`));
      const existed = await fs.stat(target);
      assert(existed.isFile());
      await fs.unlink(target);
      for (const suffix of ["-wal", "-shm"])
        await fs.rm(`${target}${suffix}`, { force: true });
      const before = await inventory(directory);
      const retainedControl = durableFiles(
        before.filter(
          (item) =>
            item.path.startsWith("data/index-control/") ||
            item.path.startsWith("data/index-work/"),
        ),
      );
      const outcome = application(directory, [
        "admin",
        "backup",
        "restore",
        "--backup",
        "/fixture/backup",
        "--confirm-restore",
      ]);
      const after = await inventory(directory);
      assert(
        !after.some((item) => item.path === missing),
        "Restore fabricated a missing effect/control ledger",
      );
      assert.deepEqual(
        durableFiles(
          after.filter(
            (item) =>
              item.path.startsWith("data/index-control/") ||
              item.path.startsWith("data/index-work/"),
          ),
        ),
        retainedControl,
      );
      assert.deepEqual(
        database(newerEpoch).delivery_dedupe,
        backups.newerDelivery,
      );
      result.cases.push({
        name: "missing-individual-ledger",
        missing,
        before,
        outcome,
        after,
        disposition:
          "No admission was started. Missing evidence stays missing; the complete quarantined newer epoch retains Unknown effects for explicit operator reconciliation.",
      });
    }
    result.status = "Passed";
    return result;
  } catch (error) {
    result.status = "Failed";
    result.error = String(error.stack ?? error);
    throw error;
  } finally {
    result.cleanup = cleanup(command, containers);
    const failedCleanup = result.cleanup.some((item) => !item.success);
    if (failedCleanup) result.status = "Failed";
    await fs.writeFile(
      path.join(root, "volume-result.json"),
      JSON.stringify(result, null, 2) + "\n",
    );
    if (failedCleanup)
      throw new Error("Volume resource cleanup failed; see the volume receipt");
  }
}
