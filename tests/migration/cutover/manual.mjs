/** Prove owner acknowledgment admits one new attempt without erasing ambiguous external effects. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

/** Exercise real HTTP admission, same-image children, acknowledgment and atomic audit persistence. */
export async function runManual(context) {
  const {
    root,
    seed,
    certificates,
    runId,
    articleId,
    fixture,
    create,
    remove,
    command,
    inventory,
  } = context;
  const directory = path.join(root, "manual-acknowledgment");
  await fs.cp(seed, directory, { recursive: true, errorOnExist: true });
  const key = path.join(root, "manual.key");
  await fs.rename(path.join(directory, "secret.key"), key);
  const ledger = `litradar-ledger-${runId}-manual`;
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
      "FIXTURE_RESPONSE=lost",
    ],
    ["ledger", "/certificates"],
    true,
  );
  command("docker", ["start", ledger]);
  const readinessDeadline = Date.now() + 20000;
  while (
    !command("docker", ["logs", ledger]).stdout.includes('"ledger-ready"')
  ) {
    assert(
      Date.now() < readinessDeadline,
      "Manual ledger did not establish isolation and TLS readiness",
    );
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  const name = `litradar-app-${runId}-manual`;
  const inspection = create(
    name,
    [
      "--mount",
      `type=bind,source=${path.join(directory, "data")},target=/app/data`,
      "--mount",
      `type=bind,source=${key},target=/keys/key,readonly`,
      "--mount",
      `type=bind,source=${certificates},target=/certificates,readonly`,
      "--env",
      "SSL_CERT_FILE=/certificates/valid-trust.pem",
      "--env",
      "SSL_CERT_DIR=/certificates/empty",
    ],
    [
      "serve",
      "--project-root",
      "/app",
      "--secret-key-file",
      "/keys/key",
      "--host",
      "127.0.0.1",
      "--port",
      "8000",
    ],
  );
  /** Keep host-side SQLite observations strictly between stopped container lifetimes. */
  async function start() {
    context.inspect(name);
    command("docker", ["start", name]);
    const deadline = Date.now() + 45000;
    while (
      command(
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
      ).status !== 0
    ) {
      assert(Date.now() < deadline, command("docker", ["logs", name]).stderr);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    command("docker", [
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
  }
  /** Prove the full service process tree stopped before opening its WAL database on Windows. */
  function stop() {
    command("docker", ["stop", "--time", "15", name]);
    const [stopped] = JSON.parse(command("docker", ["inspect", name]).stdout);
    assert.equal(stopped.State.Running, false);
    assert.equal(stopped.State.ExitCode, 0);
  }
  await start();
  /** Preserve each actual response and HTTP status, including expected rejection controls. */
  function request(method, route) {
    const result = command("docker", [
      "exec",
      name,
      "curl",
      "--silent",
      "--show-error",
      "--cookie",
      "/tmp/cookies",
      "--header",
      "Origin: http://127.0.0.1:8000",
      "--request",
      method,
      "--write-out",
      "\n%{http_code}",
      `http://127.0.0.1:8000${route}`,
    ]);
    const boundary = result.stdout.lastIndexOf("\n");
    return {
      status: Number(result.stdout.slice(boundary + 1)),
      body: JSON.parse(result.stdout.slice(0, boundary)),
    };
  }
  const snapshot = () =>
    JSON.parse(
      command(fixture, ["snapshot", path.join(directory, "data/auth.sqlite")])
        .stdout,
    );
  const sql = (statement) =>
    command(
      fixture,
      ["database", directory],
      false,
      JSON.stringify({ path: "data/auth.sqlite", sql: statement }),
    );
  /** Bound active child completion while preserving its terminal Unknown result. */
  async function terminal(id) {
    const limit = Date.now() + 60000;
    while (true) {
      const result = request("GET", `/api/tracking/push-weekly/runs/${id}`);
      assert.equal(result.status, 200);
      if (
        ["unknown", "failed", "completed", "cancelled", "timed_out"].includes(
          result.body.status,
        )
      )
        return result;
      assert(Date.now() < limit, JSON.stringify(result));
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  const admitted = request("POST", "/api/tracking/push-weekly");
  assert.equal(admitted.status, 202, JSON.stringify(admitted));
  const first = await terminal(admitted.body.job_id);
  assert.equal(first.body.status, "unknown", JSON.stringify(first));
  stop();
  const before = snapshot();
  assert(before.delivery_dedupe.some((row) => row.status === "unknown"));
  const route = `/api/tracking/push-weekly/runs/${admitted.body.job_id}/acknowledge`;
  sql(
    "CREATE TRIGGER reject_manual_audit BEFORE INSERT ON security_audit_events WHEN NEW.action='manual_push_unknown_acknowledge' BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END;",
  );
  await start();
  const blocked = request("POST", "/api/tracking/push-weekly");
  assert.equal(blocked.status, 409);
  const rejected = request("POST", route);
  assert.equal(rejected.status, 500, JSON.stringify(rejected));
  stop();
  assert.deepEqual(snapshot().delivery_runs, before.delivery_runs);
  sql("DROP TRIGGER reject_manual_audit");
  await start();
  const acknowledged = request("POST", route);
  assert.equal(acknowledged.status, 202, JSON.stringify(acknowledged));
  assert.notEqual(acknowledged.body.job_id, admitted.body.job_id);
  const second = await terminal(acknowledged.body.job_id);
  assert.equal(second.body.status, "completed", JSON.stringify(second));
  assert.equal(second.body.selected, 0);
  assert.equal(second.body.pushed, 0);
  assert.equal(second.body.total_candidates, 1);
  const duplicate = request("POST", route);
  assert.equal(duplicate.status, 409);
  stop();
  const after = snapshot();
  assert.deepEqual(after.delivery_dedupe, before.delivery_dedupe);
  assert.deepEqual(
    after.delivery_runs.find((row) => row.external_id === admitted.body.job_id),
    before.delivery_runs.find(
      (row) => row.external_id === admitted.body.job_id,
    ),
  );
  assert.equal(
    after.security_audit_events.filter(
      (row) => row.action === "manual_push_unknown_acknowledge",
    ).length,
    1,
  );
  const logs = command("docker", ["logs", ledger]);
  assert.equal(
    logs.stdout
      .split("\n")
      .filter((line) => line.includes('"synthetic-delivery"')).length,
    1,
  );
  remove(name);
  remove(ledger);
  return {
    inspection,
    admitted,
    first,
    before,
    blocked,
    rejected,
    acknowledged,
    second,
    duplicate,
    after,
    ledger: logs,
    files: await inventory(directory),
  };
}
