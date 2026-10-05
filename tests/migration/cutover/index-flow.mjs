/** Continue two ready provider worksets through final-image workers, publication and historical readers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";
import { verifyObservers, rows } from "./persistent-jobs.mjs";

/** Complete an offline same-binary index chain and return persisted content to the frozen Rust writer. */
export async function runIndexVolume(context) {
  const { copy, fixture, command, application, database, inventory, service } =
    context;
  await verifyObservers();
  const directory = await copy("two-worker-index");
  const prepared = command(fixture, ["index-notify-seed", directory]);
  const before = await inventory(directory);
  const outcome = application(directory, [
    "index",
    "--secret-key-file",
    "/keys/key",
    "--file",
    "fixture.csv",
    "--update",
    "--notify",
    "--notify-dry-run",
    "--workers",
    "1",
    "--processes",
    "2",
  ]);
  const payload = JSON.parse(outcome.stdout);
  assert.equal(payload.status, "succeeded");
  assert.equal(payload.csvs.length, 1);
  const catalog = payload.csvs[0];
  assert.equal(catalog.concurrency.child_process_count, 2);
  assert.equal(catalog.written_article_count, 2);
  assert.equal(catalog.notify_exit_code, 0);
  const content = database(directory, "data/index/fixture.sqlite");
  assert.equal(content.articles.rows.length, 2);
  assert.equal(content.article_change_events.rows.length, 0);
  const control = database(directory, "data/index-control/fixture.sqlite");
  assert.equal(control.provider_leases.rows.length, 0);
  assert.equal(control.provider_run_checkpoints.rows.length, 0);
  assert.equal(control.provider_sync_anchors.rows.length, 2);
  assert.deepEqual(
    rows(control, "provider_sync_anchors")
      .map((row) => row.catalog_id)
      .sort(),
    ["fixture-0", "fixture-1"],
  );
  const batch = database(directory, "data/index-control/index-batches.sqlite");
  const batchId = JSON.parse(prepared.stdout).batchId;
  assert.equal(
    rows(batch, "index_batches").find((row) => row.batch_id === batchId).status,
    "completed",
  );
  const completed = rows(batch, "index_batch_catalogs").find(
    (row) => row.batch_id === batchId,
  );
  assert.equal(completed.phase, "completed");
  assert.equal(completed.notify_exit_code, "0");
  assert.equal(batch.index_batch_lease.rows.length, 0);
  const worksets = await inventory(
    path.join(directory, "data/index-work/scholarly"),
  );
  assert.equal(worksets.length, 0, JSON.stringify(worksets));
  const manifestPath = path.join(
    directory,
    path.posix.relative("/fixture", catalog.manifest_path),
  );
  const manifest = await fs.readFile(manifestPath);
  const history = await fs.readFile(
    path.join(
      directory,
      "data/push_state/history/fixture",
      `${digest(manifest)}.changes.json`,
    ),
  );
  assert.deepEqual(history, manifest);
  const published = JSON.parse(manifest, (key, value, context) =>
    typeof value === "number" &&
    !Number.isSafeInteger(value) &&
    /^-?\d+$/.test(context.source)
      ? context.source
      : value,
  );
  const articleColumn = content.articles.columns.indexOf("article_id");
  assert.deepEqual(
    published.notifiable_article_ids.map(String).sort(),
    content.articles.rows
      .map((value) => JSON.parse(value)[articleColumn][1])
      .sort(),
  );
  const binary = path.resolve(
    "output/migration/execution/index-oracle/identity.exe",
  );
  const rustControl = JSON.parse(
    command(
      binary,
      [],
      false,
      JSON.stringify({
        op: "control",
        path: path.join(directory, "data/index-control/fixture.sqlite"),
        operations: [],
      }) + "\n",
    ).stdout,
  );
  const rustBatch = JSON.parse(
    command(
      binary,
      [],
      false,
      JSON.stringify({
        op: "batch",
        path: path.join(directory, "data/index-control/index-batches.sqlite"),
        operations: [],
      }) + "\n",
    ).stdout,
  );
  assert.equal(rustControl.expected.tables.provider_sync_anchors.length, 2);
  assert.equal(rustBatch.expected.tables.index_batches.length, 1);
  assert.deepEqual(
    database(directory, "data/index-control/fixture.sqlite"),
    control,
  );
  assert.deepEqual(
    database(directory, "data/index-control/index-batches.sqlite"),
    batch,
  );
  const readInput = {
    op: "content",
    path: path.join(directory, "data/index/fixture.sqlite"),
    operations: [],
  };
  const rustRead = JSON.parse(
    command(binary, [], false, JSON.stringify(readInput) + "\n").stdout,
  );
  assert.equal(rustRead.expected.tables.articles.length, 2);
  assert.deepEqual(
    database(directory, "data/index/fixture.sqlite"),
    content,
    "Historical content read changed Go state",
  );
  const vectors = JSON.parse(
    await fs.readFile("tests/migration/index/content-vectors.json", "utf8"),
  );
  const write = structuredClone(
    vectors.observations.find((item) => item.input.name === "first-and-replay")
      .input.operations[0],
  );
  write.catalog.catalog_id =
    write.batch.catalog_id =
    write.batch.journal.catalog_id =
      "rollback-added";
  write.catalog.issn = null;
  write.catalog.all_issns = [];
  write.batch.articles[0].catalog_id = "rollback-added";
  write.batch.articles[0].title = "Historical rollback added article";
  const rustWrite = JSON.parse(
    command(
      binary,
      [],
      false,
      JSON.stringify({ ...readInput, operations: [write] }) + "\n",
    ).stdout,
  );
  assert.equal(rustWrite.expected.operations[0].articles_changed, 1);
  const returned = database(directory, "data/index/fixture.sqlite");
  assert.equal(returned.articles.rows.length, 3);
  let query;
  await service(directory, async (name) => {
    query = command("docker", [
      "exec",
      name,
      "curl",
      "--fail",
      "--silent",
      "--cookie",
      "/tmp/cookies",
      "http://127.0.0.1:8000/api/articles?db=fixture.sqlite&q=rollback",
    ]);
    assert.match(query.stdout, /Historical rollback added article/);
  });
  return {
    directory,
    prepared,
    before,
    outcome,
    content,
    control,
    batch,
    worksets,
    rustControl,
    rustBatch,
    manifest: manifest.toString("utf8"),
    rustRead,
    rustWrite,
    returned,
    query,
    after: await inventory(directory),
  };
}
