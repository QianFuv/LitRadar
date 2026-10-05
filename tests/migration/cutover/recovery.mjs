/** Verify offline restore selection and legacy-byte identity through final and historical binaries. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";
import { rows } from "./persistent-jobs.mjs";

/** Exercise v1/v2 selection, unselected groups and idempotent historical raw-state import. */
export async function runRecovery(context) {
  const { copy, database, application, historical, service, inventory, seed } =
    context;
  const outcomes = [];
  for (const version of [1, 2]) {
    const directory = await copy(`restore-v${version}`);
    const backup = path.join(directory, "backup");
    await fs.mkdir(backup);
    const source = "tests/migration/storage/fixtures/backup-v2";
    const manifest = JSON.parse(
      await fs.readFile(path.join(source, "manifest.json"), "utf8"),
    );
    manifest.version = version;
    manifest.selection.metadata = version === 2;
    manifest.components = manifest.components.filter(
      (item) => version === 2 || item.kind !== "metadata",
    );
    for (const component of manifest.components) {
      const body = await fs.readFile(path.join(source, component.path));
      assert.equal(body.length, component.size);
      assert.equal(digest(body), component.sha256);
      const destination = path.join(backup, component.path);
      assert(
        path.resolve(destination).startsWith(path.resolve(backup) + path.sep),
      );
      await fs.mkdir(path.dirname(destination), { recursive: true });
      await fs.writeFile(destination, body);
    }
    await fs.writeFile(
      path.join(backup, "manifest.json"),
      JSON.stringify(manifest) + "\n",
    );
    const metadata = await inventory(path.join(directory, "data/meta"));
    const expectedDatabases = {};
    await fs.mkdir(path.join(directory, "expected"));
    for (const component of manifest.components.filter((item) =>
      item.kind.endsWith("_database"),
    )) {
      const relative = `expected/${component.kind}.sqlite`;
      await fs.copyFile(
        path.join(backup, component.path),
        path.join(directory, relative),
      );
      expectedDatabases[component.path] = database(directory, relative);
    }
    const key = digest(await fs.readFile(path.join(seed, "secret.key")));
    const verify = application(directory, [
      "admin",
      "backup",
      "verify",
      "--backup",
      "/fixture/backup",
    ]);
    const rust = historical(directory, [
      "admin",
      "backup",
      "verify",
      "--backup",
      backup,
    ]);
    const restore = application(directory, [
      "admin",
      "backup",
      "restore",
      "--backup",
      "/fixture/backup",
      "--confirm-restore",
    ]);
    assert.equal(database(directory).version, 20);
    assert.equal(database(directory, "data/index/metadata.sqlite").version, 9);
    for (const [relative, expected] of Object.entries(expectedDatabases))
      assert.deepEqual(database(directory, `data/${relative}`), expected);
    for (const [kind, prefix] of [
      ["metadata", "meta/"],
      ["push_state", "push_state/"],
    ]) {
      if (kind === "metadata" && version === 1) continue;
      const expected = manifest.components
        .filter((item) => item.kind === kind)
        .map((item) => ({
          path: item.path.slice(prefix.length),
          sha256: item.sha256,
        }))
        .sort((first, second) => first.path.localeCompare(second.path));
      assert.deepEqual(
        await inventory(path.join(directory, "data", prefix)),
        expected,
      );
    }
    assert(
      !(await fs.readdir(path.join(directory, "data/index"))).includes(
        "full-stack.sqlite",
      ),
    );
    const afterMeta = await inventory(path.join(directory, "data/meta"));
    if (version === 1) assert.deepEqual(afterMeta, metadata);
    else
      assert.deepEqual(
        afterMeta.map((item) => item.path),
        ["nested/catalog.csv"],
      );
    assert.equal(digest(await fs.readFile(path.join(seed, "secret.key"))), key);
    outcomes.push({
      name: `restore-v${version}`,
      origin:
        version === 1
          ? "v1 manifest assembled from fixed original Rust component bytes"
          : "fixed original Rust v2 components",
      verify,
      rust,
      restore,
      metadata,
      expectedDatabases,
      after: await inventory(directory),
    });
  }
  const selected = await copy("restore-omitted-groups");
  const create = application(selected, [
    "admin",
    "backup",
    "create",
    "--output",
    "/fixture/backup",
  ]);
  const beforeIndexes = await inventory(path.join(selected, "data/index"));
  await fs.writeFile(
    path.join(selected, "data/push_state/newer.json"),
    '{"retained":true}\n',
  );
  const beforePush = await inventory(path.join(selected, "data/push_state"));
  const restore = application(selected, [
    "admin",
    "backup",
    "restore",
    "--backup",
    "/fixture/backup",
    "--confirm-restore",
  ]);
  assert.deepEqual(
    await inventory(path.join(selected, "data/index")),
    beforeIndexes,
  );
  assert.deepEqual(
    await inventory(path.join(selected, "data/push_state")),
    beforePush,
  );
  outcomes.push({
    name: "restore-omitted-index-and-push-groups",
    create,
    restore,
    beforeIndexes,
    beforePush,
  });

  const directory = await copy("legacy-raw-import");
  const corpus = JSON.parse(
    await fs.readFile("tests/migration/delivery/legacy-vectors.json", "utf8"),
  );
  const file = corpus.cases.find((item) => item.name === "fresh-and-idempotent")
    .steps[0].files[0];
  await fs.writeFile(path.join(directory, file.path), file.body);
  const rawHash = digest(Buffer.from(file.body));
  const first = await service(directory);
  const imported = database(directory);
  const checkpoint = rows(imported, "delivery_checkpoints").find(
    (row) => row.db_name === "fixture.sqlite",
  );
  assert.equal(checkpoint.legacy_source_hash, rawHash);
  const rust = historical(directory, [
    "scheduler",
    "validate",
    "--secret-key-file",
    path.join(seed, "secret.key"),
  ]);
  assert.deepEqual(
    database(directory).delivery_checkpoints,
    imported.delivery_checkpoints,
  );
  const second = await service(directory);
  assert.deepEqual(
    database(directory).delivery_checkpoints,
    imported.delivery_checkpoints,
  );
  assert.equal(
    digest(await fs.readFile(path.join(directory, file.path))),
    rawHash,
  );
  outcomes.push({
    name: "legacy-raw-import-return-to-rust-and-go",
    rawHash,
    first,
    imported,
    rust,
    second,
  });
  return outcomes;
}
