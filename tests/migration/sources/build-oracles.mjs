/** Build observations from the unchanged source crate and narrowly exposed private helpers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Run a bounded compiler with the original Windows Rust compiler environment. */
function run(command, args) {
  const environment = { ...process.env };
  if (process.platform === "win32") {
    delete environment.CC;
    delete environment.CXX;
  }
  const result = spawnSync(command, args, {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 180000,
    maxBuffer: 32 * 1024 * 1024,
    env: environment,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr + result.stdout);
  return result;
}

const built = run("cargo", [
  "build",
  "-p",
  "litradar-sources",
  "--locked",
  "--message-format=json",
]);
const artifacts = built.stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line))
  .filter((item) => item.reason === "compiler-artifact" && !item.profile.test);
const externs = [];
const dependencies = [];
for (const name of [
  "litradar_sources",
  "litradar_provider",
  "litradar_domain",
  "serde_json",
  "chrono",
  "reqwest",
  "tracing",
  "serde",
  "rusqlite",
  "sha2",
  "getrandom",
  "regex",
]) {
  const selectedVersion = {
    getrandom: "0.4.3",
    sha2: "0.11.0",
    rusqlite: "0.37.0",
    serde: "1.0.228",
  }[name];
  const files = [
    ...new Set(
      artifacts
        .filter((item) => item.target.name === name)
        .filter(
          (item) =>
            !selectedVersion ||
            item.package_id.endsWith(`@${selectedVersion}`) ||
            item.package_id.endsWith(`#${selectedVersion}`),
        )
        .flatMap((item) =>
          item.filenames.filter((file) => file.endsWith(".rlib")),
        ),
    ),
  ];
  assert.equal(files.length, 1, `Ambiguous artifact for ${name}`);
  externs.push("--extern", `${name}=${files[0]}`);
  dependencies.push({
    name,
    path: files[0],
    sha256: digest(await fs.readFile(files[0])),
  });
}
const copiedSources = [];
await fs.mkdir("output/migration/execution/sources-oracle", {
  recursive: true,
});
const originalPath = "crates/litradar-sources/src/http_retry.rs";
const original = await fs.readFile(originalPath, "utf8");
assert.equal(original.split("\nfn parse_retry_after(").length, 2);
const exposed = original.replace(
  "\nfn parse_retry_after(",
  "\npub fn parse_retry_after(",
);
await fs.writeFile(
  "output/migration/execution/sources-oracle/http_retry.rs",
  exposed,
);
copiedSources.push({
  path: originalPath,
  original_sha256: digest(Buffer.from(original)),
  exposed_sha256: digest(Buffer.from(exposed)),
});
const schedulerPath = "crates/litradar-sources/src/scholarly.rs";
const schedulerOriginal = await fs.readFile(schedulerPath, "utf8");
const rowsDeclaration = schedulerOriginal.match(
  /^pub\(crate\) const CROSSREF_ROWS: usize = \d+;$/m,
)?.[0];
assert.ok(rowsDeclaration);
await fs.writeFile(
  "output/migration/execution/sources-oracle/workset_rows.rs",
  rowsDeclaration + "\n",
);
copiedSources.push({
  path: schedulerPath,
  original_sha256: digest(Buffer.from(schedulerOriginal)),
  exposed_sha256: digest(Buffer.from(rowsDeclaration + "\n")),
  seam: "Unchanged CROSSREF_ROWS declaration.",
});
const schedulerStart = schedulerOriginal.indexOf(
  "const OPENALEX_DEFAULT_REMAINING_CREDITS:",
);
const schedulerEnd = schedulerOriginal.indexOf(
  "#[derive(Clone)]\nstruct SemanticScholarScheduler {",
);
assert.ok(schedulerStart >= 0 && schedulerEnd > schedulerStart);
const schedulerCopied = schedulerOriginal.slice(schedulerStart, schedulerEnd);
await fs.writeFile(
  "output/migration/execution/sources-oracle/schedulers.rs",
  schedulerCopied,
);
copiedSources.push({
  path: schedulerPath,
  original_sha256: digest(Buffer.from(schedulerOriginal)),
  exposed_sha256: digest(Buffer.from(schedulerCopied)),
  start: schedulerStart,
  end: schedulerEnd,
});
const zjlibPath = "crates/litradar-sources/src/zjlib.rs";
const zjlibOriginal = await fs.readFile(zjlibPath, "utf8");
const zjlibObserver = await fs.readFile(
  "tests/migration/sources/zjlib-observer.rs",
  "utf8",
);
assert.equal(zjlibOriginal.split("fn current_unix_time() -> i64 {").length, 2);
const zjlibExposed =
  zjlibOriginal.replace(
    "fn current_unix_time() -> i64 {",
    "fn original_current_unix_time() -> i64 {",
  ) +
  "\n" +
  zjlibObserver;
await fs.writeFile(
  "output/migration/execution/sources-oracle/zjlib.rs",
  zjlibExposed,
);
copiedSources.push({
  path: zjlibPath,
  original_sha256: digest(Buffer.from(zjlibOriginal)),
  exposed_sha256: digest(Buffer.from(zjlibExposed)),
  observer_sha256: digest(Buffer.from(zjlibObserver)),
  seam: "Replace only the Unix-seconds clock; append observations of original private helpers.",
});
const providersPath = "crates/litradar-sources/src/providers.rs";
const providersOriginal = await fs.readFile(providersPath, "utf8");
const providerRanges = [
  [
    "const SCHOLARLY_ANCHOR_VERSION: u32 = 1;",
    "#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]",
  ],
  [
    "/// Derive the unchanged canonical issue fingerprint",
    "fn scholarly_issue_anchor(article:",
  ],
  ["fn scholarly_issue_anchor_from_fields(", "fn scholarly_issue_is_older("],
  ["fn valid_year(", "fn encode_scholarly_anchor("],
  [
    "/// Validate the stable anchor format shared",
    "fn encode_scholarly_checkpoint(",
  ],
  ["/// Read the preferred publication date", "fn crossref_authors("],
  ["fn json_text(", "fn strip_markup("],
].map(([startMarker, endMarker]) => {
  const start = providersOriginal.indexOf(startMarker);
  const end = providersOriginal.indexOf(endMarker, start);
  assert.ok(
    start >= 0 && end > start,
    `Missing original anchor helper ${startMarker}`,
  );
  return { start, end };
});
const providersCopied = providerRanges
  .map(({ start, end }) => providersOriginal.slice(start, end))
  .join("\n");
await fs.writeFile(
  "output/migration/execution/sources-oracle/workset_providers.rs",
  providersCopied,
);
copiedSources.push({
  path: providersPath,
  original_sha256: digest(Buffer.from(providersOriginal)),
  exposed_sha256: digest(Buffer.from(providersCopied)),
  ranges: providerRanges,
});
const worksetPath = "crates/litradar-sources/src/crossref_workset.rs";
const worksetOriginal = await fs.readFile(worksetPath, "utf8");
const worksetObserver = await fs.readFile(
  "tests/migration/sources/workset-observer.rs",
  "utf8",
);
const worksetExposed = worksetOriginal + "\n" + worksetObserver;
await fs.writeFile(
  "output/migration/execution/sources-oracle/crossref_workset.rs",
  worksetExposed,
);
copiedSources.push({
  path: worksetPath,
  original_sha256: digest(Buffer.from(worksetOriginal)),
  exposed_sha256: digest(Buffer.from(worksetExposed)),
  observer_sha256: digest(Buffer.from(worksetObserver)),
  seam: "Append observation wrapper only; retain original workset implementation.",
});
const indexProviderPath = "crates/litradar-sources/src/providers.rs";
const indexProviderOriginal = await fs.readFile(indexProviderPath, "utf8");
const indexObserver = await fs.readFile(
  "tests/migration/sources/index-observer.rs",
  "utf8",
);
assert.equal(
  indexProviderOriginal.split("fn current_utc_date() -> Option<String> {")
    .length,
  2,
);
const indexProviderExposed =
  indexProviderOriginal.replace(
    "fn current_utc_date() -> Option<String> {",
    "fn original_current_utc_date() -> Option<String> {",
  ) +
  "\n" +
  indexObserver;
await fs.writeFile(
  "output/migration/execution/sources-oracle/index_providers.rs",
  indexProviderExposed,
);
copiedSources.push({
  path: indexProviderPath,
  original_sha256: digest(Buffer.from(indexProviderOriginal)),
  exposed_sha256: digest(Buffer.from(indexProviderExposed)),
  observer_sha256: digest(Buffer.from(indexObserver)),
  seam: "Copy complete providers module, replace only wall-clock UTC date entry with a fixed test date, append observers.",
});
for (const name of [
  "transport",
  "provider",
  "scholarly",
  "scheduler",
  "cnki",
  "zjlib",
  "workset",
  "access",
  "index",
]) {
  const source = `tests/migration/sources/${name}-oracle.rs`;
  const binary = `output/migration/execution/sources-${name}-oracle.exe`;
  const args = [
    "--edition=2024",
    source,
    "-L",
    "dependency=target/debug/deps",
    ...externs,
    "-o",
    binary,
  ];
  run("rustc", args);
  await fs.writeFile(
    `output/migration/execution/t05-${name}-oracle-build.json`,
    JSON.stringify(
      {
        builder_sha256: digest(
          await fs.readFile("tests/migration/sources/build-oracles.mjs"),
        ),
        command: "rustc",
        args,
        source_sha256: digest(await fs.readFile(source)),
        binary_sha256: digest(await fs.readFile(binary)),
        dependencies,
        copiedSources,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`Built original Rust source ${name} oracle`);
}
