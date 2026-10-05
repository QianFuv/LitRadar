/** Exercise real historical layouts and refuse unsupported state without discarding operator data. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";

/** Compare canonical content independently of rebuildable FTS segment representation. */
function canonical(snapshot) {
  return Object.fromEntries(
    Object.entries(snapshot).filter(
      ([name]) =>
        !["schema", "version"].includes(name) &&
        !name.startsWith("sqlite_stat") &&
        !name.startsWith("article_fts") &&
        !name.startsWith("article_search"),
    ),
  );
}

/** Verify historical optimizer entrances, future-schema refusal and exact managed metadata preservation. */
export async function runFormats(context) {
  const { copy, database, application, service, inventory } = context;
  const outcomes = [];
  const fixtures = JSON.parse(
    await fs.readFile("tests/migration/storage/index-vectors.json", "utf8"),
  );
  for (const version of [4, 5, 6, 7, 8, 9]) {
    const directory = await copy(`content-version-${version}`);
    const filename = path.join(directory, "data/index/historical.sqlite");
    await fs.copyFile(
      `tests/migration/storage/fixtures/content-v${version}.sqlite.fixture`,
      filename,
    );
    const rawBefore = digest(await fs.readFile(filename));
    assert.equal(
      rawBefore,
      fixtures.cases.find((item) => item.version === version).sha256,
    );
    const before = database(directory, "data/index/historical.sqlite");
    assert.equal(before.version, version);
    const result = application(
      directory,
      ["admin", "index", "optimize-storage", "--confirm-index-maintenance"],
      true,
    );
    const payload = JSON.parse(result.stdout);
    const after = database(directory, "data/index/historical.sqlite");
    if (version < 6) {
      assert.notEqual(result.status, 0);
      assert.equal(payload.error.code, "unsupported_schema");
      assert.equal(digest(await fs.readFile(filename)), rawBefore);
      assert.deepEqual(after, before);
    } else {
      assert.equal(result.status, 0, result.stderr);
      assert.equal(payload.status, "optimized");
      assert.equal(after.version, 9);
      assert.deepEqual(canonical(after), canonical(before));
    }
    for (const marker of [
      ".litradar-index-maintenance.json",
      ".litradar-index-staging",
      ".litradar-index-rollback",
    ])
      assert(
        !(await fs.readdir(path.join(directory, "data"))).includes(marker),
      );
    outcomes.push({
      name: `historical-content-v${version}`,
      before,
      result,
      after,
      files: await inventory(directory),
    });
    if (version < 6) {
      const upgraded = await copy(`ordinary-upgrade-v${version}`, directory);
      const startup = application(upgraded, [
        "scheduler",
        "validate",
        "--secret-key-file",
        "/keys/key",
      ]);
      const migrated = database(upgraded, "data/index/historical.sqlite");
      assert.equal(migrated.version, 9);
      const tables = [
        "journals",
        "issues",
        "articles",
        "article_identity_keys",
        "article_listing",
        "article_change_events",
      ];
      const actual = tables.map((table) =>
        migrated[table].rows
          .map((row) =>
            JSON.stringify(
              JSON.parse(row).map(([kind, value]) =>
                kind === "null"
                  ? null
                  : ["integer", "real"].includes(kind)
                    ? Number(value)
                    : value,
              ),
            ),
          )
          .sort(),
      );
      assert.deepEqual(
        actual,
        fixtures.cases
          .find((item) => item.version === version)
          .canonical.map((records) => [...records].sort()),
      );
      assert.equal(migrated.article_retraction_dois.rows.length, 0);
      outcomes.push({
        name: `ordinary-startup-v${version}-to-v9`,
        startup,
        migrated,
      });
    }
  }
  for (const [name, filename, version] of [
    ["auth", "data/auth.sqlite", 21],
    ["index", "data/index/full-stack.sqlite", 10],
  ]) {
    const directory = await copy(`future-${name}`);
    database(directory, filename, `PRAGMA user_version=${version}`);
    const before = database(directory, filename);
    const result = application(
      directory,
      ["scheduler", "validate", "--secret-key-file", "/keys/key"],
      true,
    );
    assert.notEqual(result.status, 0);
    assert.deepEqual(database(directory, filename), before);
    database(
      directory,
      filename,
      `PRAGMA user_version=${name === "auth" ? 20 : 9}`,
    );
    const positive = application(directory, [
      "scheduler",
      "validate",
      "--secret-key-file",
      "/keys/key",
    ]);
    outcomes.push({ name: `future-${name}-refusal`, before, result, positive });
  }
  const directory = await copy("managed-metadata");
  const files = (await fs.readdir("assets/meta"))
    .filter((name) => name.endsWith(".csv"))
    .sort();
  const adopted = files[0],
    custom = files[1],
    missing = files[2];
  await fs.copyFile(
    path.join("assets/meta", adopted),
    path.join(directory, "data/meta", adopted),
  );
  const customBytes = Buffer.concat([
    await fs.readFile(path.join("assets/meta", custom)),
    Buffer.from("\n\n"),
  ]);
  await fs.writeFile(path.join(directory, "data/meta", custom), customBytes);
  const first = await service(directory);
  assert.equal(
    digest(await fs.readFile(path.join(directory, "data/meta", custom))),
    digest(customBytes),
  );
  assert.equal(
    digest(await fs.readFile(path.join(directory, "data/meta", adopted))),
    digest(await fs.readFile(path.join("assets/meta", adopted))),
  );
  assert.equal(
    digest(await fs.readFile(path.join(directory, "data/meta", missing))),
    digest(await fs.readFile(path.join("assets/meta", missing))),
  );
  const beforeRestart = await inventory(path.join(directory, "data/meta"));
  const second = await service(directory);
  assert.deepEqual(
    await inventory(path.join(directory, "data/meta")),
    beforeRestart,
  );
  outcomes.push({
    name: "managed-meta-created-adopted-customized",
    adopted,
    custom,
    missing,
    first,
    second,
    bytes: beforeRestart,
  });
  return outcomes;
}
