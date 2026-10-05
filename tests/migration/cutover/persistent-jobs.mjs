/** Continue retained Rust scheduler histories and CFP captures through the final Go application. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";

/** Validate retained observer artifacts against the committed handover registry. */
export async function verifyObservers() {
  const registry = JSON.parse(
    await fs.readFile("tests/migration/cutover/inputs.json", "utf8"),
  );
  for (const item of registry.files)
    assert.equal(
      digest(await fs.readFile(item.path)),
      item.sha256,
      `Changed handover input: ${item.path}`,
    );
  return registry;
}

/** Decode the type-preserving database snapshot for selected business-row assertions. */
export function rows(snapshot, table) {
  return snapshot[table].rows.map((encoded) =>
    Object.fromEntries(
      JSON.parse(encoded).map(([kind, value], index) => [
        snapshot[table].columns[index],
        kind === "null" ? null : value,
      ]),
    ),
  );
}

/** Resume four scheduler lifecycle categories without permitting external provider requests. */
export async function runSchedulerVolumes(context) {
  const { copy, database, service, command, inventory, fixture } = context;
  await verifyObservers();
  const corpus = JSON.parse(
    await fs.readFile("tests/migration/scheduler/storage-vectors.json", "utf8"),
  );
  const binary = path.resolve(
    "output/migration/execution/scheduler-oracle/storage.exe",
  );
  const outcomes = [];
  const now = new Date();
  assert(
    !(now.getUTCMonth() === 0 && now.getUTCDate() <= 2) &&
      !(now.getUTCMonth() === 11 && now.getUTCDate() === 31),
    "Choose a nontriggering synthetic cron away from the current observation window",
  );
  for (const [name, count, expected, id] of [
    ["coalesced-success", 2, "success", "1"],
    ["expired-scheduled-claim-true", 4, "success", "2"],
    ["running-expiry-unknown", 4, "unknown", "1"],
    ["expired-manual-false-false", 2, "cancelled", "1"],
  ]) {
    const directory = await copy(`scheduler-${name}`);
    const steps = structuredClone(
      corpus.cases.find((item) => item.name === name).steps.slice(0, count),
    );
    steps[0].cron = "0 0 1 1 *";
    steps[0].job = {
      kind: "index",
      metadata_file: "fixture.csv",
      notify: false,
      push: false,
    };
    const prepared = command(fixture, ["index-seed", directory]);
    const filename = path.join(directory, "data/auth.sqlite");
    const seed = command(
      binary,
      [],
      false,
      JSON.stringify({ path: filename, resume: true, steps }) + "\n",
    );
    const before = database(directory);
    const poll = async (container) => {
      const deadline = Date.now() + 30000;
      while (true) {
        const response = JSON.parse(
          command("docker", [
            "exec",
            container,
            "curl",
            "--fail",
            "--silent",
            "--cookie",
            "/tmp/cookies",
            "http://127.0.0.1:8000/api/admin/scheduler/status",
          ]).stdout,
        );
        const state = response.recent_runs;
        if (
          state.some((row) => String(row.id) === id && row.status === expected)
        )
          break;
        assert(Date.now() < deadline, JSON.stringify(state));
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
    };
    const first = await service(directory, poll);
    const after = database(directory);
    const run = rows(after, "scheduled_task_runs").find((row) => row.id === id);
    assert.equal(run.status, expected);
    if (expected === "unknown") {
      const original = rows(before, "scheduled_task_runs").find(
        (row) => row.id === id,
      );
      assert.equal(run.worker_id, original.worker_id);
      assert.equal(run.started_at, original.started_at);
    }
    const restart = await service(directory, poll);
    assert.deepEqual(
      database(directory).scheduled_task_runs,
      after.scheduled_task_runs,
      "Restart repeated terminal scheduler work",
    );
    const rust = command(
      binary,
      [],
      false,
      JSON.stringify({
        path: filename,
        resume: true,
        steps: [
          { op: "finish", run: Number(id), now: 2000000000 },
          { op: "renew", run: Number(id), now: 2000000000 },
        ],
      }) + "\n",
    );
    assert(
      JSON.parse(rust.stdout).every((item) => item.outcome === false),
      "Old owner overwrote final Go outcome",
    );
    outcomes.push({
      name,
      prepared,
      seed,
      before,
      first,
      after,
      restart,
      rust,
      files: await inventory(directory),
    });
  }
  return outcomes;
}

/** Use a production registry identity while preserving the frozen historical HTML and capture format. */
function adaptCapture(original, config) {
  const prefix = new URL("../", config.discoveryUrl).pathname;
  const origin = new URL(config.discoveryUrl).origin;
  const rewrite = (value) =>
    typeof value === "string"
      ? value
          .replaceAll("https://example.org/calls", config.discoveryUrl)
          .replaceAll(
            "https://example.org/collections/",
            `${origin}${prefix}collections/`,
          )
          .replaceAll("'/collections/", `'${prefix}collections/`)
          .replaceAll("Example Journal", config.journalTitle)
      : Array.isArray(value)
        ? value.map(rewrite)
        : value && typeof value === "object"
          ? Object.fromEntries(
              Object.entries(value).map(([key, child]) => [
                key,
                rewrite(child),
              ]),
            )
          : value;
  const input = rewrite(structuredClone(original));
  input.config = config;
  const seed = JSON.parse(input.seed);
  for (const item of seed.sources) item.catalogIds = config.catalogIds;
  input.seed = JSON.stringify(seed);
  input.capture.result.sourceKey = config.sourceKey;
  return input;
}

/** Reuse complete and partial original-document captures offline, then hand Go evidence back to Rust. */
export async function runCfpVolumes(context) {
  const { root, copy, database, application, command, inventory, fixture } =
    context;
  await verifyObservers();
  const corpus = JSON.parse(
    await fs.readFile("tests/migration/cfp/worker-vectors.json", "utf8"),
  );
  const registry = JSON.parse(
    await fs.readFile("assets/cfp/sources.json", "utf8"),
  );
  const config = registry.find(
    (item) => item.sourceKey === "journal:issn-2096-5796",
  );
  assert(config);
  const binary = path.resolve(
    "output/migration/execution/cfp-oracle/worker.exe",
  );
  const outcomes = [];
  for (const name of ["full-complete", "full-partial"]) {
    const directory = await copy(`cfp-${name}`);
    const capture = path.join(directory, "captures");
    await fs.mkdir(capture);
    const input = adaptCapture(
      corpus.cases.find((item) => item.name === name).input,
      config,
    );
    const prepared = command(
      fixture,
      ["cfp-seed", directory],
      false,
      JSON.stringify(input),
    );
    await fs.copyFile(
      path.join(directory, "data/auth.sqlite"),
      path.join(capture, "state.sqlite"),
    );
    input.directory = capture;
    input.resume = true;
    const rustSeed = JSON.parse(
      command(binary, [], false, JSON.stringify(input) + "\n").stdout,
    );
    await fs.copyFile(
      path.join(capture, "state.sqlite"),
      path.join(directory, "data/auth.sqlite"),
    );
    await fs.copyFile(
      "assets/meta/ccf_computer_journals.csv",
      path.join(directory, "data/meta/ccf_computer_journals.csv"),
    );
    const before = database(directory);
    const result = application(
      directory,
      [
        "cfp",
        "refresh",
        "--catalog-id",
        config.catalogIds[0],
        "--full-text",
        "--capture-dir",
        "/fixture/captures",
        "--resume-captures",
        "--source-timeout",
        "30",
        "--timeout",
        "60",
      ],
      true,
    );
    assert(result.stdout.trim(), JSON.stringify(result));
    const payload = JSON.parse(result.stdout);
    assert.equal(result.status === 0, name === "full-complete");
    assert.equal(payload.sources.length, 1);
    assert.equal(payload.recoveredNotices, 1);
    assert.equal(payload.updatedNotices, 1);
    assert.equal(payload[name === "full-complete" ? "success" : "partial"], 1);
    const after = database(directory);
    const sources = rows(after, "cfp_sources");
    const source = sources.find((item) => item.source_key === config.sourceKey);
    const previous = rows(before, "cfp_sources").find(
      (item) => item.source_key === config.sourceKey,
    );
    assert.equal(
      source.status,
      name === "full-complete" ? "success" : "failed",
    );
    assert.equal(BigInt(source.revision), BigInt(previous.revision) + 1n);
    assert.equal(BigInt(source.generation), BigInt(previous.generation) + 1n);
    assert.equal(source.lease_expires_at, null);
    assert.equal(source.content_hash, digest(Buffer.from(source.capture)));
    assert.deepEqual(after.cfp_seed_imports, before.cfp_seed_imports);
    const currentCapture = JSON.parse(
      await fs.readFile(
        path.join(capture, "journal_issn-2096-5796.json"),
        "utf8",
      ),
    );
    assert.equal(currentCapture.result.sourceKey, config.sourceKey);
    const returned = path.join(root, `cfp-returned-${name}`);
    await fs.mkdir(returned);
    await fs.copyFile(
      path.join(directory, "data/auth.sqlite"),
      path.join(returned, "state.sqlite"),
    );
    await fs.copyFile(
      path.join(capture, "journal_issn-2096-5796.json"),
      path.join(returned, "journal_issn-2096-5796.json"),
    );
    const resumed = { ...input, directory: returned, resume: true };
    delete resumed.capture;
    const rust = JSON.parse(
      command(binary, [], false, JSON.stringify(resumed) + "\n").stdout,
    );
    assert.deepEqual(rust.originals, rustSeed.originals);
    assert.equal(rust.outcome.status, rustSeed.outcome.status);
    outcomes.push({
      name,
      prepared,
      before,
      result,
      after,
      rustSeed,
      rust,
      capture: currentCapture,
      files: await inventory(directory),
    });
  }
  return outcomes;
}
