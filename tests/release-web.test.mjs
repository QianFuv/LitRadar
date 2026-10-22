/** Verify release consumers receive only the tested source's complete static export. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { generateCspManifest } from "../scripts/generate-csp.mjs";
import { stageWebAssets } from "../scripts/stage-web-assets.mjs";
import {
  prepareReleaseWeb,
  verifyReleaseWeb,
  stageReleaseWeb,
} from "../scripts/release-web.mjs";

const SOURCE = "a".repeat(40);

/** Create a real staged export with CSP, gzip and hidden/underscored assets. */
async function fixture(context) {
  const root = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-release-web-"),
  );
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  const output = path.join(root, "app/out");
  await fs.mkdir(path.join(output, "_next/static"), { recursive: true });
  await fs.writeFile(
    path.join(output, "index.html"),
    "<script>ready=true;</script>",
  );
  await fs.writeFile(path.join(output, "404.html"), "missing");
  await fs.writeFile(path.join(output, "_next/static/chunk.js"), "ready=true;");
  await fs.writeFile(path.join(output, ".asset"), "hidden but required");
  await generateCspManifest(output);
  const staged = await stageWebAssets(root);
  const compressedPath = path.join(staged, "index.html.gz");
  const compressed = await fs.readFile(compressedPath);
  compressed[9] = process.platform === "win32" ? 3 : 0;
  await fs.writeFile(compressedPath, compressed);
  const bundle = path.join(root, "bundle");
  await prepareReleaseWeb(staged, bundle, SOURCE);
  return {
    root,
    staged,
    bundle,
    exportDirectory: path.join(bundle, "app/out"),
  };
}

test("both release consumers retain exact tested assets, CSP and compressed bytes", async (context) => {
  const { root, staged, bundle, exportDirectory } = await fixture(context);
  assert.equal(await verifyReleaseWeb(bundle, SOURCE), exportDirectory);
  const windowsRoot = path.join(root, "windows");
  await fs.mkdir(windowsRoot);
  const windowsExport = await stageReleaseWeb(bundle, SOURCE, windowsRoot);
  for (const file of [
    "index.html",
    "index.html.gz",
    "csp-hashes.json",
    "_next/static/chunk.js.gz",
    ".asset",
  ]) {
    const expected = await fs.readFile(path.join(staged, file));
    assert.deepEqual(
      await fs.readFile(path.join(exportDirectory, file)),
      expected,
    );
    assert.deepEqual(
      await fs.readFile(path.join(windowsExport, file)),
      expected,
    );
  }
  await assert.rejects(
    verifyReleaseWeb(bundle, "b".repeat(40)),
    /source differs/,
  );
  await assert.rejects(prepareReleaseWeb(staged, bundle, SOURCE), {
    code: "EEXIST",
  });
});

for (const populated of [false, true]) {
  test(`staging preserves an existing ${populated ? "populated" : "empty"} embed directory`, async (context) => {
    const { root, bundle } = await fixture(context);
    const consumer = path.join(root, "consumer");
    const destination = path.join(consumer, "internal/webassets/export");
    await fs.mkdir(destination, { recursive: true });
    if (populated)
      await fs.writeFile(path.join(destination, "sentinel"), "preserve");
    await assert.rejects(stageReleaseWeb(bundle, SOURCE, consumer), {
      code: "EEXIST",
    });
    assert.deepEqual(
      await fs.readdir(destination),
      populated ? ["sentinel"] : [],
    );
    if (populated)
      assert.equal(
        await fs.readFile(path.join(destination, "sentinel"), "utf8"),
        "preserve",
      );
  });
}

for (const mutation of ["changed", "missing", "extra", "linked"]) {
  test(`verification rejects ${mutation} assets before compilation`, async (context) => {
    const { root, bundle, exportDirectory } = await fixture(context);
    const asset = path.join(exportDirectory, "_next/static/chunk.js");
    if (mutation === "changed") await fs.writeFile(asset, "modified");
    if (mutation === "missing") await fs.unlink(asset);
    if (mutation === "extra")
      await fs.writeFile(path.join(exportDirectory, "extra"), "unlisted");
    if (mutation === "linked") {
      const outside = path.join(root, "outside");
      await fs.mkdir(outside);
      await fs.symlink(
        outside,
        path.join(exportDirectory, "link"),
        process.platform === "win32" ? "junction" : "dir",
      );
    }
    await assert.rejects(
      verifyReleaseWeb(bundle, SOURCE),
      /inventory differs|symbolic links/,
    );
  });
}

test("release wiring uses the immutable same-run artifact and bypasses only duplicate frontend builds", async () => {
  const frontend = await fs.readFile(".github/workflows/frontend.yaml", "utf8");
  const release = await fs.readFile(".github/workflows/release.yaml", "utf8");
  const ci = await fs.readFile(".github/workflows/ci.yaml", "utf8");
  assert(
    frontend.indexOf("pnpm test:e2e:full-stack") <
      frontend.indexOf("release-web.mjs prepare"),
  );
  assert.match(frontend, /include-hidden-files: true/);
  assert.match(
    frontend,
    /web_artifact_id:.*steps.web_export.outputs.artifact-id/,
  );
  assert.match(ci, /web_artifact_id:.*needs.frontend.outputs.web_artifact_id/);
  assert.equal(
    (release.match(/artifact-ids:.*inputs.web_artifact_id/g) ?? []).length,
    2,
  );
  assert.match(
    release,
    /build-contexts: frontend-build=.\/release-results\/web/,
  );
  assert(!release.includes("pnpm build"));
  assert.match(release, /package-windows.mjs.*--verified-web/);
  assert.match(release, /release-web.mjs verify/);
  assert.match(release, /node tests\/container-smoke.mjs/);
  assert.match(release, /node scripts\/package-windows.mjs/);
});
