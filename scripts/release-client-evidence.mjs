/** Verify client artifacts against the exact source and transferred byte inventory. */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import { clientAssetName } from "./package-client.mjs";

/** Reject missing packages, changed bytes and cross-source client artifacts. */
export async function verifyClientEvidence(root, version, source) {
  assert.match(source, /^[a-f0-9]{40}$/);
  for (const [target, platform] of Object.entries({
    web: "linux/amd64",
    windows: "windows/amd64",
    android: "android/arm64",
  })) {
    const directory = path.join(root, `client-${target}-release`);
    const manifest = JSON.parse(
      fs.readFileSync(path.join(directory, "ci-manifest.json"), "utf8"),
    );
    assert.equal(manifest.sourceSha, source, "Client source differs");
    assert.equal(manifest.status, "success", "Client build must succeed");
    assert.equal(manifest.stage, "build-acceptance");
    assert.deepEqual(manifest.platforms, [platform]);
    assert.deepEqual(manifest.missing, []);
    const files = new Set();
    for (const file of manifest.files) {
      assert(!files.has(file.path), "Duplicate client inventory entry");
      files.add(file.path);
      assert(
        !path.isAbsolute(file.path) &&
          !file.path.includes("\\") &&
          !file.path.split("/").some((part) => [".", "..", ""].includes(part)),
        "Client path escaped artifact root",
      );
      let filename = directory;
      for (const part of file.path.split("/")) {
        filename = path.join(filename, part);
        assert(
          !fs.lstatSync(filename).isSymbolicLink(),
          "Client evidence refuses links",
        );
      }
      assert.equal(
        fs.statSync(filename).size,
        file.bytes,
        "Client size differs",
      );
      const digest = createHash("sha256");
      for await (const chunk of fs.createReadStream(filename))
        digest.update(chunk);
      assert.equal(
        digest.digest("hex"),
        file.sha256,
        "Client checksum differs",
      );
    }
    const name = `assets/${clientAssetName(version, target)}`;
    if (target === "android") {
      assert(
        files.has("signing-status.json"),
        "Android signing evidence missing",
      );
      const status = JSON.parse(
        fs.readFileSync(path.join(directory, "signing-status.json"), "utf8"),
      ).android;
      assert(
        ["signed", "omitted"].includes(status),
        "Android release must be signed or explicitly omitted",
      );
      assert.equal(
        files.has(name),
        status === "signed",
        "Android signing/package mismatch",
      );
      assert.equal(
        fs.existsSync(path.join(directory, name)),
        status === "signed",
        "Unrecorded Android APK",
      );
    } else {
      assert(files.has(name), "Required client package missing");
    }
    if (target === "web") {
      for (const filename of [
        "image-client-web.tar",
        "image-client-web.json",
        "smoke.json",
      ]) {
        assert(
          files.has(filename),
          `Client image evidence missing: ${filename}`,
        );
      }
      const smoke = JSON.parse(
        fs.readFileSync(path.join(directory, "smoke.json"), "utf8"),
      );
      const [image] = JSON.parse(
        fs.readFileSync(path.join(directory, "image-client-web.json"), "utf8"),
      );
      verifyClientImage(image, smoke, version, source);
    }
  }
}

/** Require the accepted image identity, platform, version and immutable source. */
export function verifyClientImage(image, smoke, version, source) {
  assert.equal(smoke.status, "passed", "Client image smoke must pass");
  assert.match(smoke.imageId, /^sha256:[a-f0-9]{64}$/);
  assert.equal(
    image.Id,
    smoke.imageId,
    "Client image differs from tested bytes",
  );
  assert.equal(image.Os, "linux");
  assert.equal(image.Architecture, "amd64");
  assert.equal(
    image.Config.Labels["org.opencontainers.image.revision"],
    source,
  );
  assert.equal(
    image.Config.Labels["org.opencontainers.image.version"],
    version,
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const root = "release-results/clients";
  const version = process.env.RELEASE_VERSION;
  const source = process.env.RELEASE_SOURCE_SHA || process.env.GITHUB_SHA;
  if (process.argv[2] === "image") {
    verifyClientImage(
      JSON.parse(
        fs.readFileSync("release-results/loaded-client-image.json", "utf8"),
      )[0],
      JSON.parse(
        fs.readFileSync(`${root}/client-web-release/smoke.json`, "utf8"),
      ),
      version,
      source,
    );
  } else {
    await verifyClientEvidence(root, version, source);
  }
}
