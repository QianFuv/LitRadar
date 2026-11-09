/** Exercise mandatory client artifacts, release signing and exact-source byte transfer. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  clientAssetName,
  packageWindowsPortable,
  stageClient,
} from "../scripts/package-client.mjs";
import {
  verifyClientEvidence,
  verifyClientImage,
} from "../scripts/release-client-evidence.mjs";
import { assertAndroidSigner } from "../scripts/verify-android-signing.mjs";
import {
  clientVersions,
  MANIFEST_VERSION_PLACEHOLDER,
  windowsResources,
} from "../scripts/client-version.mjs";
import { writeCiReport } from "../../../scripts/ci-report.mjs";
import {
  releaseAssetNames,
  validateAssets,
} from "../../../scripts/release-assets.mjs";

const VERSION = "0.3.0";
const SOURCE = "a".repeat(40);
const IMAGE = {
  Id: `sha256:${"b".repeat(64)}`,
  Os: "linux",
  Architecture: "amd64",
  Config: {
    Labels: {
      "org.opencontainers.image.revision": SOURCE,
      "org.opencontainers.image.version": VERSION,
    },
  },
};

/** Create a private temporary workspace removed after each test. */
function temporaryRoot(context) {
  const root = fs.mkdtempSync(
    path.join(os.tmpdir(), "litradar-client-release-"),
  );
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

/** Produce independent source-bound client evidence with signed or explicitly omitted Android. */
async function evidenceFixture(root, android) {
  for (const [target, platform] of Object.entries({
    web: "linux/amd64",
    windows: "windows/amd64",
    android: "android/arm64",
  })) {
    const directory = path.join(root, `client-${target}-release`);
    fs.mkdirSync(directory, { recursive: true });
    if (target !== "android" || android === "signed") {
      fs.mkdirSync(path.join(directory, "assets"), { recursive: true });
      fs.writeFileSync(
        path.join(directory, "assets", clientAssetName(VERSION, target)),
        `tested ${target}`,
      );
    }
    if (target === "android")
      fs.writeFileSync(
        path.join(directory, "signing-status.json"),
        JSON.stringify({ android }),
      );
    if (target === "web") {
      fs.writeFileSync(
        path.join(directory, "image-client-web.tar"),
        "tested image bytes",
      );
      fs.writeFileSync(
        path.join(directory, "image-client-web.json"),
        JSON.stringify([IMAGE]),
      );
      fs.writeFileSync(
        path.join(directory, "smoke.json"),
        JSON.stringify({ status: "passed", imageId: IMAGE.Id }),
      );
    }
    await writeCiReport(directory, ["."], {
      sourceSha: SOURCE,
      platforms: [platform],
      status: "success",
      stage: "build-acceptance",
      requireFiles: true,
    });
  }
}

test("client packaging accepts only the contracted nonempty outputs and supported platforms", (context) => {
  const root = temporaryRoot(context);
  const source = path.join(root, "apps/client/bin");
  fs.mkdirSync(source, { recursive: true });
  const output = path.join(root, "staged");
  fs.writeFileSync(
    path.join(source, "litradar-client_0.3.0_windows_amd64.zip"),
    "verified portable zip",
  );
  stageClient("windows", VERSION, root, output);
  assert.equal(
    fs.readFileSync(
      path.join(output, "assets/litradar-client_0.3.0_windows_amd64.zip"),
      "utf8",
    ),
    "verified portable zip",
  );
  fs.writeFileSync(path.join(source, "litradar-client-web"), "");
  assert.throws(() => stageClient("web", VERSION, root, output), /nonempty/);
  for (const target of ["ios", "darwin", "linux", "android-amd64"])
    assert.throws(() => clientAssetName(VERSION, target), /Unsupported/);
});

test(
  "the Windows client ships as a portable zip with one versioned folder",
  {
    skip:
      process.platform !== "win32" && "Windows packaging uses the Windows tar",
  },
  (context) => {
    const root = temporaryRoot(context);
    const bin = path.join(root, "apps/client/bin");
    fs.mkdirSync(bin, { recursive: true });
    fs.writeFileSync(path.join(bin, "litradar-client.exe"), "portable exe");
    fs.writeFileSync(path.join(root, "LICENSE"), "license text");
    const archive = packageWindowsPortable(VERSION, root);
    assert.equal(
      archive,
      path.join(bin, "litradar-client_0.3.0_windows_amd64.zip"),
    );
    const entries = execFileSync(
      path.join(process.env.SystemRoot, "System32", "tar.exe"),
      ["-tf", archive],
      { encoding: "utf8" },
    )
      .split(/\r?\n/)
      .filter((entry) => entry && !entry.endsWith("/"))
      .sort();
    assert.deepEqual(entries, [
      "litradar-client_0.3.0_windows_amd64/LICENSE",
      "litradar-client_0.3.0_windows_amd64/litradar-client.exe",
    ]);
    assert.deepEqual(fs.readdirSync(bin).sort(), [
      "litradar-client.exe",
      "litradar-client_0.3.0_windows_amd64.zip",
    ]);
  },
);

for (const android of ["signed", "omitted"]) {
  test(`client evidence accepts ${android} Android and rejects changed or missing mandatory bytes`, async (context) => {
    const root = temporaryRoot(context);
    await evidenceFixture(root, android);
    await verifyClientEvidence(root, VERSION, SOURCE);
    await assert.rejects(
      verifyClientEvidence(root, VERSION, "c".repeat(40)),
      /source differs/,
    );
    const filename = path.join(
      root,
      "client-windows-release/assets",
      clientAssetName(VERSION, "windows"),
    );
    fs.appendFileSync(filename, "tampered");
    await assert.rejects(
      verifyClientEvidence(root, VERSION, SOURCE),
      /size differs/,
    );
    fs.unlinkSync(filename);
    await assert.rejects(verifyClientEvidence(root, VERSION, SOURCE), /ENOENT/);
  });
}

test("Android quality signing cannot enter a release and omission cannot hide an APK", async (context) => {
  const root = temporaryRoot(context);
  await evidenceFixture(root, "quality");
  await assert.rejects(
    verifyClientEvidence(root, VERSION, SOURCE),
    /signed or explicitly omitted/,
  );
  await evidenceFixture(root, "omitted");
  const directory = path.join(root, "client-android-release/assets");
  fs.mkdirSync(directory);
  fs.writeFileSync(
    path.join(directory, clientAssetName(VERSION, "android")),
    "debug APK",
  );
  await assert.rejects(
    verifyClientEvidence(root, VERSION, SOURCE),
    /Unrecorded Android/,
  );
});

test("loaded client image must match smoke-tested bytes, target, source and version", () => {
  const smoke = { status: "passed", imageId: IMAGE.Id };
  verifyClientImage(IMAGE, smoke, VERSION, SOURCE);
  for (const image of [
    { ...IMAGE, Id: `sha256:${"c".repeat(64)}` },
    { ...IMAGE, Architecture: "arm64" },
    { ...IMAGE, Os: "windows" },
    { ...IMAGE, Config: { Labels: {} } },
  ])
    assert.throws(() => verifyClientImage(image, smoke, VERSION, SOURCE));
  assert.throws(() =>
    verifyClientImage(IMAGE, { ...smoke, status: "failed" }, VERSION, SOURCE),
  );
});

test("a valid debug APK signature cannot substitute for the configured release certificate", () => {
  const expected = "a".repeat(64);
  assertAndroidSigner(
    `Signer #1 certificate SHA-256 digest: ${expected}\n`,
    expected,
  );
  assert.throws(
    () =>
      assertAndroidSigner(
        `Signer #1 certificate SHA-256 digest: ${"b".repeat(64)}`,
        expected,
      ),
    /release signing certificate/,
  );
  assert.throws(
    () => assertAndroidSigner("", expected),
    /release signing certificate/,
  );
});

test("checksum manifest includes signed Android when present and always requires Web and Windows clients", (context) => {
  const root = temporaryRoot(context);
  const names = releaseAssetNames(VERSION, false, true);
  const lines = names.slice(0, -1).map((name) => {
    const bytes = `tested ${name}`;
    fs.writeFileSync(path.join(root, name), bytes);
    return `${createHash("sha256").update(bytes).digest("hex")}  ${name}\n`;
  });
  fs.writeFileSync(path.join(root, "SHA256SUMS"), lines.slice(0, -1).join(""));
  assert.throws(() => validateAssets(root, VERSION), /expected platforms/);
  fs.writeFileSync(path.join(root, "SHA256SUMS"), lines.join(""));
  assert.equal(validateAssets(root, VERSION).length, 6);
  fs.unlinkSync(path.join(root, clientAssetName(VERSION, "web")));
  assert.throws(() => validateAssets(root, VERSION), /ENOENT/);
});

test("workflows preserve isolated client quality and the release signing omission contract", () => {
  const client = fs.readFileSync(".github/workflows/client.yaml", "utf8");
  const frontend = fs.readFileSync(".github/workflows/frontend.yaml", "utf8");
  const release = fs.readFileSync(".github/workflows/release.yaml", "utf8");
  for (const command of [
    "task build:server",
    "task package:windows",
    "task package:android",
  ])
    assert(client.includes(command));
  assert(client.includes("go-task/task/v3/cmd/task@v3.54.0"));
  assert(
    client.includes("JAVA_HOME_21_X64") && client.includes("ndk;28.2.13676358"),
  );
  assert(
    client.includes("Android APK omitted") &&
      client.includes("GITHUB_STEP_SUMMARY"),
  );
  assert(client.includes("verify-android-signing.mjs"));
  assert(!frontend.includes("Check Wails web host"));
  assert(client.includes("Check Wails web host"));
  for (const obsolete of [
    "web_artifact_id",
    "release-web.mjs",
    "--verified-web",
    "frontend-build=",
  ])
    assert(!release.includes(obsolete));
  assert(release.includes("needs: [context, windows, linux, client]"));
  assert(release.includes("node tests/compose-smoke.mjs"));
});

test("client build metadata derives its version only from VERSION", () => {
  const readClientFile = (relativePath) =>
    fs.readFileSync(new URL(`../${relativePath}`, import.meta.url), "utf8");
  const semanticVersions = (text) =>
    text.match(/\b\d+\.\d+\.\d+(?:\.\d+)?\b/g) ?? [];
  for (const relativePath of ["build/config.yml", "build/windows/info.json"])
    assert.deepEqual(
      semanticVersions(readClientFile(relativePath)),
      [],
      relativePath,
    );
  const manifest = readClientFile("build/windows/wails.exe.manifest");
  assert.deepEqual(semanticVersions(manifest), ["6.0.0.0"]);
  assert(
    manifest.includes(
      `name="com.qianfuv.litradar" version="${MANIFEST_VERSION_PLACEHOLDER}"`,
    ),
  );
  const gradle = readClientFile("build/android/app/build.gradle");
  assert.match(gradle, /versionCode = releaseVersion\.code\b/);
  assert.match(gradle, /versionName = releaseVersion\.name\b/);
  assert.match(
    gradle,
    /rootProject\.file\('\.\.\/\.\.\/\.\.\/\.\.\/VERSION'\)/,
  );
  const taskfile = readClientFile("Taskfile.yml");
  assert(
    taskfile.includes("node scripts/client-version.mjs windows-resources"),
  );
  assert(!taskfile.includes("-manifest build/windows/"));
  assert(!taskfile.includes("-info build/windows/"));
});

test("client version identifiers are derived from VERSION", () => {
  assert.deepEqual(clientVersions("9.8.7\n"), {
    versionName: "9.8.7",
    versionCode: 90807,
    fileVersion: "9.8.7.0",
  });
  assert.deepEqual(clientVersions("0.1.0"), {
    versionName: "0.1.0",
    versionCode: 100,
    fileVersion: "0.1.0.0",
  });
  assert.equal(clientVersions("65535.99.99").versionCode, 655359999);
  for (const malformed of [
    "1.2",
    "v1.2.3",
    "1.2.3-rc.1",
    "01.2.3",
    "1.100.0",
    "1.2.100",
    "65536.0.0",
  ])
    assert.throws(() => clientVersions(malformed), /VERSION/, malformed);
  const versions = clientVersions(
    fs.readFileSync(new URL("../../../VERSION", import.meta.url), "utf8"),
  );
  const resources = windowsResources(
    fs.readFileSync(
      new URL("../build/windows/info.json", import.meta.url),
      "utf8",
    ),
    fs.readFileSync(
      new URL("../build/windows/wails.exe.manifest", import.meta.url),
      "utf8",
    ),
    clientVersions("9.8.7"),
  );
  const info = JSON.parse(resources.info);
  assert.deepEqual(info.fixed, {
    file_version: "9.8.7.0",
    product_version: "9.8.7.0",
  });
  for (const strings of Object.values(info.info)) {
    assert.equal(strings.FileVersion, "9.8.7");
    assert.equal(strings.ProductVersion, "9.8.7");
    assert.equal(strings.ProductName, "LitRadar");
  }
  assert(
    resources.manifest.includes(
      'name="com.qianfuv.litradar" version="9.8.7.0"',
    ),
  );
  assert(!resources.manifest.includes(MANIFEST_VERSION_PLACEHOLDER));
  assert.match(versions.fileVersion, /^\d+\.\d+\.\d+\.0$/);
  assert.throws(
    () => windowsResources('{"info": {}}', "<assembly/>", versions),
    /FILE_VERSION/,
  );
});
