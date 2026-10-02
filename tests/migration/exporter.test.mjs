/** Exercise the frozen exporter's destructive-boundary guards as an actual process. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { BASELINE, loadOracle } from "./oracle.mjs";

test("exporter refuses unsafe or reused destinations without changing their contents", async (context) => {
  const oracle = await loadOracle(BASELINE);
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-exporter-guard-"),
  );
  context.after(async () => {
    assert.equal(path.dirname(root), path.resolve(os.tmpdir()));
    await fs.rm(root, { recursive: true });
  });
  const executable = path.join(
    oracle.directory,
    oracle.manifest.binaries.exporter,
  );
  /**
   * Require a real process refusal with bounded execution.
   * @param {string[]} args - Exporter arguments.
   * @returns {void}
   */
  function refuse(args) {
    const result = spawnSync(executable, args, {
      cwd: root,
      encoding: "utf8",
      timeout: 10_000,
      maxBuffer: 1024 * 1024,
    });
    assert.ifError(result.error);
    assert.equal(result.status, 1, result.stdout);
  }
  await fs.writeFile(path.join(root, "sentinel"), "preserve");
  refuse([]);
  refuse([root, root]);
  refuse([root]);
  await fs.writeFile(
    path.join(root, ".litradar-migration-fixture"),
    "wrong marker\n",
  );
  refuse([root]);
  await fs.writeFile(
    path.join(root, ".litradar-migration-fixture"),
    "litradar-migration-fixture-v1\n",
  );
  await fs.mkdir(path.join(root, "fixtures"));
  await fs.writeFile(
    path.join(root, "fixtures", "existing"),
    "do not overwrite",
  );
  refuse([root]);
  assert.equal(
    await fs.readFile(path.join(root, "fixtures", "existing"), "utf8"),
    "do not overwrite",
  );
  assert.equal(
    await fs.readFile(path.join(root, "sentinel"), "utf8"),
    "preserve",
  );
  assert.deepEqual(await fs.readdir(path.join(root, "fixtures")), ["existing"]);
  const alias = path.join(root, "linked-root");
  await fs.symlink(root, alias, "junction");
  refuse([alias]);
  await fs.unlink(alias);
});
