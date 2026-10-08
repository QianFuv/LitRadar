/** Verify production staging retains exact assets and fails closed before replacing output. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { gunzipSync } from "node:zlib";
import { generateCspManifest } from "../scripts/generate-csp.mjs";
import { stageWebAssets } from "../scripts/stage-web-assets.mjs";

/** Create an isolated repository-shaped export and arrange task-owned cleanup. */
async function fixture(context) {
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-embed-stage-"),
  );
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  const output = path.join(root, "app", "out");
  await fs.mkdir(path.join(output, "_next", "static"), { recursive: true });
  await fs.writeFile(
    path.join(output, "index.html"),
    "<script>ready=true;</script>",
  );
  await fs.writeFile(path.join(output, "404.html"), "missing");
  await fs.writeFile(
    path.join(output, "_next", "static", "chunk.js"),
    "ready=true;",
  );
  await generateCspManifest(output);
  return { root, output };
}

test("staging includes underscored assets, exact CSP and reproducible gzip without stale files", async (context) => {
  const { root, output } = await fixture(context);
  const destination = await stageWebAssets(root);
  const asset = path.join(destination, "_next", "static", "chunk.js");
  assert.equal(await fs.readFile(asset, "utf8"), "ready=true;");
  const compressed = await fs.readFile(`${asset}.gz`);
  assert.equal(gunzipSync(compressed).toString(), "ready=true;");
  assert.equal(
    await fs.readFile(path.join(destination, "csp-hashes.json"), "utf8"),
    await fs.readFile(path.join(output, "csp-hashes.json"), "utf8"),
  );
  await fs.writeFile(path.join(destination, "retired.js"), "stale");
  await stageWebAssets(root);
  assert.deepEqual(await fs.readFile(`${asset}.gz`), compressed);
  await assert.rejects(fs.stat(path.join(destination, "retired.js")), {
    code: "ENOENT",
  });
});

test("stale, missing and malformed manifests preserve previously staged assets", async (context) => {
  const { root, output } = await fixture(context);
  const destination = await stageWebAssets(root);
  const manifest = path.join(output, "csp-hashes.json");
  const duplicate = (await fs.readFile(manifest, "utf8")).replace(
    '"version": 1',
    '"version": 1, "version": 1',
  );
  for (const content of ["{}", "not-json", duplicate, null]) {
    if (content === null) await fs.unlink(manifest);
    else await fs.writeFile(manifest, content);
    await assert.rejects(stageWebAssets(root));
    assert.equal(
      await fs.readFile(path.join(destination, "index.html"), "utf8"),
      "<script>ready=true;</script>",
    );
  }
});

test("staging refuses linked destination directories", async (context) => {
  const { root } = await fixture(context);
  const parent = path.join(root, "internal", "webassets");
  await fs.mkdir(parent, { recursive: true });
  const outside = path.join(root, "outside");
  await fs.mkdir(outside);
  await fs.writeFile(path.join(outside, "sentinel"), "preserve");
  await fs.symlink(
    outside,
    path.join(parent, "export"),
    process.platform === "win32" ? "junction" : "dir",
  );
  await assert.rejects(stageWebAssets(root), /Unsafe embed destination/);
  assert.equal(
    await fs.readFile(path.join(outside, "sentinel"), "utf8"),
    "preserve",
  );
});
