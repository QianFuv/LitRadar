/** Compile unchanged original CFP assertions with observation-only public-call wrappers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/cfp-oracle/storage-build.json",
    "utf8",
  ),
);
const originalPath = "crates/litradar-sources/tests/cfp.rs",
  wrapperPath = "tests/migration/cfp/source-observer.rs";
const original = await fs.readFile(originalPath, "utf8"),
  wrapper = await fs.readFile(wrapperPath, "utf8");
const replaced = original
  .replace(
    /use litradar_sources::cfp::\{[\s\S]*?\};/,
    "use litradar_sources::cfp::{cfp_source_registry,CfpAdapter,CfpDocument,CfpSourceConfig,CfpSourceError,CfpTransport,CfpUrlRule};",
  )
  .replaceAll(
    /include_str!\("([^\"]+)"\)/g,
    (_, file) =>
      `include_str!(${JSON.stringify(path.resolve(path.dirname(originalPath), file).replaceAll("\\", "/"))})`,
  );
assert.notEqual(original, replaced);
const generated = "output/migration/execution/cfp-oracle/source-tests.rs",
  binary = `output/migration/execution/cfp-oracle/source${process.platform === "win32" ? ".exe" : ""}`;
await fs.writeFile(generated, replaced + "\n" + wrapper);
const args = [
  "--edition=2024",
  "--test",
  generated,
  "-L",
  "dependency=target/debug/deps",
  ...build.dependencies.flatMap((item) => [
    "--extern",
    `${item.name}=${item.path}`,
  ]),
  "-o",
  binary,
];
const env = { ...process.env };
if (process.platform === "win32") {
  delete env.CC;
  delete env.CXX;
}
const result = spawnSync("rustc", args, {
  env,
  encoding: "utf8",
  windowsHide: true,
  timeout: 180000,
  maxBuffer: 16 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stdout + result.stderr);
const inputs = [...build.inputs];
for (const file of [
  originalPath,
  wrapperPath,
  "crates/litradar-sources/assets/cfp-sources.json",
  ...["springer", "elsevier", "keai"].map(
    (name) => `crates/litradar-sources/tests/fixtures/cfp-${name}.txt`,
  ),
])
  inputs.push({ path: file, sha256: digest(await fs.readFile(file)) });
await fs.writeFile(
  "output/migration/execution/cfp-oracle/source-build.json",
  JSON.stringify(
    {
      binary,
      binary_sha256: digest(await fs.readFile(binary)),
      builder_sha256: digest(
        await fs.readFile("tests/migration/cfp/build-source.mjs"),
      ),
      command: "rustc",
      args,
      inputs,
      dependencies: build.dependencies,
    },
    null,
    2,
  ) + "\n",
);
