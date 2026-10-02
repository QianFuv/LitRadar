/**
 * Freeze source-derived public surfaces and framework-discovered test identities.
 * Counts detect omissions; this inventory never substitutes for behavioral parity.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { BASELINE, WORKSPACE_ROOT, digest } from "./oracle.mjs";

const HTTP_METHODS = new Set([
  "get",
  "post",
  "put",
  "delete",
  "patch",
  "head",
  "options",
]);
const SOURCE_ROUTES = "crates/litradar-api/src/routes/mod.rs";
const SOURCE_SCHEMA = "app/lib/generated/openapi.json";
const SOURCE_MCP = "crates/litradar-api/src/mcp.rs";
const SOURCE_SETTINGS =
  "crates/litradar-storage/src/business/runtime_settings.rs";

/**
 * Read the approved committed source, independently of later application changes.
 * @param {string} filename - Repository-relative tracked source.
 * @returns {string} Baseline source text.
 */
function baselineText(filename) {
  return execFileSync("git", ["show", `${BASELINE}:${filename}`], {
    cwd: WORKSPACE_ROOT,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
}

/**
 * Assign existing behavior tests to the implementation phases that own their domain.
 * Broad integration tests retain multiple owners instead of claiming early closure.
 * @param {string} packageName - Rust package name.
 * @param {string} name - Full test identity including its module.
 * @returns {{groups: string[], owners: string[], firstProof: string[], finalIntegration: string, disposition: string, reason: string}} Migration disposition.
 */
function classify(packageName, name) {
  const selected = new Map();
  const rules = [
    [/litradar-storage::meta::/, ["M11"], ["T04"]],
    [/business::runtime_settings::/, ["M05", "M23"], ["T03", "T11"]],
    [/business::security_audit::/, ["M05", "M23"], ["T03", "T11"]],
    [/password|argon2|pbkdf2/, ["M06"], ["T02", "T03"]],
    [/secret|encrypt|key_rotation|key_file/, ["M06"], ["T02", "T03"]],
    [/auth|user|invite|access_token|session_cookie/, ["M05"], ["T03"]],
    [/cfp|full_text|rendered|obscura/, ["M21", "M22"], ["T09"]],
    [
      /process_tree|parent_process|supervis|child_process|descendant/,
      ["M20"],
      ["T02", "T08", "T09"],
    ],
    [/scheduler|scheduled|cron|coalesc|daylight|timezone/, ["M19"], ["T08"]],
    [
      /delivery|pushplus|notification|dedupe|unknown_attempt|checkpoint_import/,
      ["M17", "M18"],
      ["T07"],
    ],
    [/ai_|fallback|recommend/, ["M17"], ["T07"]],
    [/crossref_workset|partition|cursor_cache/, ["M16"], ["T05", "T06"]],
    [
      /cnki|zjlib|captcha|provider_proxy|fulltext|article_access/,
      ["M14"],
      ["T03", "T05"],
    ],
    [
      /scholarly|crossref|openalex|semantic_scholar|provider|retry_after|response_body/,
      ["M13"],
      ["T05"],
    ],
    [/weekly|manifest/, ["M09", "M15"], ["T04", "T06"]],
    [/favorite|folder|citation|bibtex|endnote|ris_/, ["M10"], ["T04"]],
    [/managed_meta|meta_bundle|bundled_meta/, ["M11"], ["T04"]],
    [/backup|restore|storage_optim|maintenance/, ["M12"], ["T04"]],
    [/migrat|schema|preflight/, ["M07"], ["T03", "T04", "T07"]],
    [/search|fts|rating|article_query|normaliz/, ["M08"], ["T04"]],
    [/announcement|metadata|journal|issue|article/, ["M09"], ["T04"]],
    [
      /worker_protocol|content_commit|outbox|index|identity|anchor|batch/,
      ["M15"],
      ["T06"],
    ],
    [/mcp/, ["M03"], ["T02", "T10"]],
    [/openapi|route|http|response|extractor|cors/, ["M02"], ["T10"]],
    [
      /static|development_mode|readiness|service|runtime|heartbeat|shutdown/,
      ["M04", "M23"],
      ["T11"],
    ],
    [
      /logging|observability|redact|trace|correlation|admission|executor|rate_limit/,
      ["M23"],
      ["T03", "T10", "T11"],
    ],
  ];
  for (const [pattern, groups, owners] of rules) {
    if (pattern.test(name))
      for (const group of groups) {
        const existing = selected.get(group) ?? new Set();
        owners.forEach((owner) => existing.add(owner));
        selected.set(group, existing);
      }
  }
  const defaults = {
    litradar: ["M01", "M23", "T11"],
    "litradar-api": ["M02", "T10"],
    "litradar-auth": ["M05", "M06", "T03"],
    "litradar-cli": ["M01", "T11"],
    "litradar-domain": [
      "M02",
      "M15",
      "T02",
      "T03",
      "T04",
      "T05",
      "T06",
      "T07",
      "T08",
      "T09",
      "T10",
    ],
    "litradar-index": ["M15", "T06"],
    "litradar-provider": ["M13", "T05"],
    "litradar-recommend": ["M17", "T07"],
    "litradar-sources": ["M13", "M14", "T05"],
    "litradar-storage": ["M07", "M09", "T04"],
    "litradar-worker": ["M17", "M18", "M19", "M20", "M21", "T07", "T08", "T09"],
  };
  assert(defaults[packageName], `Unmapped package: ${packageName}`);
  if (
    selected.size === 0 ||
    ["litradar", "litradar-api", "litradar-cli"].includes(packageName)
  ) {
    const fallback = defaults[packageName];
    for (const group of fallback.filter((value) => value.startsWith("M"))) {
      selected.set(
        group,
        new Set([
          ...(selected.get(group) ?? []),
          ...fallback.filter((value) => value.startsWith("T")),
        ]),
      );
    }
  }
  const owners = [
    ...new Set([...selected.values()].flatMap((value) => [...value])),
  ].sort();
  const leavesByGroup = {
    M01: ["C01"],
    M02: ["C02", "C32", "C33"],
    M03: ["C03"],
    M04: ["C29"],
    M05: ["C04"],
    M06: ["C05", "C06"],
    M07: ["C07", "C34"],
    M08: ["C08", "C09"],
    M09: ["C10", "C32"],
    M10: ["C11"],
    M11: ["C12"],
    M12: ["C13", "C14"],
    M13: ["C15", "C16"],
    M14: ["C17", "C18", "C33"],
    M15: ["C19", "C20"],
    M16: ["C21"],
    M17: ["C22"],
    M18: ["C23", "C24", "C34"],
    M19: ["R03", "R04", "R05"],
    M20: ["C25", "R02"],
    M21: ["C26"],
    M22: ["C27"],
    M23: ["C28", "C29", "C30"],
  };
  return {
    groups: [...selected.keys()].sort(),
    owners,
    leaves: [
      ...new Set([...selected.keys()].flatMap((group) => leavesByGroup[group])),
    ].sort(),
    firstProof: owners.map((owner) => `${owner}/V${owner.slice(1)}`),
    finalIntegration:
      "T11; T12 packaged boundaries; T13 persistent handover; T14 exact final artifact as applicable",
    disposition: /direct_output_macros|source_allowlist/.test(name)
      ? "replaced"
      : "migrated",
    reason: /direct_output_macros|source_allowlist/.test(name)
      ? "Replace Rust-specific source inspection with equivalent Go output/redaction boundary assertions"
      : "Retain the named business assertion; each listed task proves its owned portion before final integration",
  };
}

/**
 * Produce an exhaustive explicit route/schema registry and a preserved test identity list.
 * @returns {Promise<object>} Machine-readable baseline inventory.
 */
export async function buildInventory() {
  const surfaces = JSON.parse(
    await fs.readFile(
      path.join(WORKSPACE_ROOT, "tests/data/migration/surfaces.json"),
      "utf8",
    ),
  );
  const schema = JSON.parse(baselineText(SOURCE_SCHEMA));
  const routeSource = baselineText(SOURCE_ROUTES);
  const registrations = [];
  for (const route of routeSource.matchAll(
    /\.route\(\s*"([^"]+)"\s*,([\s\S]*?)(?=\.route\(|\n\})/g,
  )) {
    const routePath = route[1].startsWith("/health/")
      ? route[1]
      : `/api${route[1]}`;
    for (const match of route[2].matchAll(
      /(?:axum::routing::|\.)(get|post|put|delete)\(\s*([\w:]+)\s*\)/g,
    )) {
      registrations.push({
        method: match[1].toUpperCase(),
        path: routePath,
        handler: match[2],
      });
    }
  }
  const operations = [];
  for (const [route, methods] of Object.entries(schema.paths)) {
    for (const [method, contract] of Object.entries(methods)) {
      if (!HTTP_METHODS.has(method)) continue;
      const registered = registrations.find(
        (entry) =>
          entry.path === route && entry.method === method.toUpperCase(),
      );
      assert(registered, `Documented but unregistered: ${method} ${route}`);
      operations.push({
        ...registered,
        operationId: contract.operationId,
        contract,
        group: "M02",
        leaf: "C02",
        owner: "T10",
        firstProof: "V10",
        finalIntegration: "T11/G2",
        parity: "Not Run",
        comparison: "status/headers/ordered payload; no implicit normalization",
      });
    }
  }
  assert.equal(operations.length, 86);
  assert.equal(
    registrations.length,
    operations.length,
    "Undocumented route or duplicate registration",
  );
  assert.equal(
    new Set(registrations.map((row) => `${row.method} ${row.path}`)).size,
    86,
  );
  const tools = [
    ...baselineText(SOURCE_MCP).matchAll(/#\[tool\(\s*name\s*=\s*"([^"]+)"/g),
  ].map((match) => match[1]);
  assert.equal(tools.length, 13);
  const keys = [
    ...baselineText(SOURCE_SETTINGS)
      .split("pub const fn as_str(self)")[1]
      .split("/// Resolve")[0]
      .matchAll(/Self::(\w+)\s*=>\s*"([^"]+)"/g),
  ].map((match) => ({
    variant: match[1],
    field: match[2],
    owner: "T03",
    firstProof: "V03",
    finalIntegration: "T10/T11",
  }));
  assert.equal(keys.length, 20);
  const listingPath = path.join(
    WORKSPACE_ROOT,
    "output/migration/execution/nextest-list.json",
  );
  const listingBytes = await fs.readFile(listingPath);
  const listing = JSON.parse(
    listingBytes.toString("utf8").replace(/^\uFEFF/, ""),
  );
  const resultBytes = await fs.readFile(
    path.join(WORKSPACE_ROOT, "output/migration/execution/v01-nextest.log"),
  );
  const resultText = resultBytes.toString("utf8");
  assert(
    /1168 tests run: 1168 passed.*13 skipped/.test(resultText),
    "Baseline execution evidence missing or failed",
  );
  const tests = [];
  for (const [suiteName, suite] of Object.entries(listing["rust-suites"])) {
    assert.equal(suite.status, "listed");
    for (const [name, properties] of Object.entries(suite.testcases)) {
      const isInfrastructure =
        properties.ignored && /helper|fixture/.test(name);
      tests.push({
        id: `${suiteName}::${name}`,
        package: suite["package-name"],
        target: suite.kind,
        baselinePlatform: `${process.platform}-${process.arch}`,
        ignored: properties.ignored,
        ...classify(suite["package-name"], `${suiteName}::${name}`),
        baselineStatus: properties.ignored
          ? "Not Run as standalone test"
          : "Passed: v01-nextest.log",
        ignoredReason: isInfrastructure
          ? "Child-process entry invoked by its supervising test; preserve as test infrastructure"
          : properties.ignored
            ? "Explicit ignored integration/benchmark; run separately when its documented environment is prepared"
            : null,
      });
    }
  }
  assert.equal(tests.length, listing["test-count"]);
  const sourceFiles = execFileSync(
    "git",
    ["ls-tree", "-r", "--name-only", BASELINE, "crates"],
    { cwd: WORKSPACE_ROOT, encoding: "utf8" },
  )
    .trim()
    .split("\n")
    .filter((filename) => filename.endsWith(".rs"));
  const sourceOnly = [];
  for (const filename of sourceFiles) {
    const source = baselineText(filename);
    for (const match of source.matchAll(
      /((?:#\[[\s\S]*?\]\s*)+)(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(/g,
    )) {
      if (!/#\[(?:tokio::)?test(?:\(|\])/.test(match[1])) continue;
      const name = match[2];
      if (
        tests.some(
          (entry) =>
            entry.package === filename.split("/")[1] &&
            entry.id.endsWith(`::${name}`),
        )
      )
        continue;
      sourceOnly.push({
        source: filename,
        line: source.slice(0, match.index).split("\n").length,
        name,
        attributes: match[1].trim(),
        baselineStatus: "Not Run on this target; source inventory only",
        ...classify(filename.split("/")[1], `${filename}::${name}`),
      });
    }
  }
  return {
    format: 1,
    baseline: BASELINE,
    status: "Inventory; candidate parity remains Not Run",
    provenance: [SOURCE_SCHEMA, SOURCE_ROUTES, SOURCE_MCP, SOURCE_SETTINGS].map(
      (filename) => ({
        path: filename,
        sha256: digest(Buffer.from(baselineText(filename))),
      }),
    ),
    operations: operations.sort((left, right) =>
      `${left.path} ${left.method}`.localeCompare(
        `${right.path} ${right.method}`,
        "en",
      ),
    ),
    schemas: schema.components.schemas,
    mcp: tools.map((name) => ({
      name,
      contract: surfaces.mcp.tools.find((tool) => tool.name === name),
      group: "M03",
      leaf: "C03",
      owners: ["T02", "T10"],
      firstProof: ["T02/V02 transport", "T10/V10 tool behavior"],
      finalIntegration: "T11",
    })),
    cli: surfaces.cli.map((entry) => ({
      ...entry,
      group: "M01",
      leaf: "C01",
      owner: "T11",
      firstProof: "V11",
      finalIntegration: "T14 exact binary",
      kind:
        entry.args.length === 1 ||
        [
          "admin",
          "admin secrets",
          "admin backup",
          "admin index",
          "cfp",
          "scheduler",
        ].includes(entry.args.slice(0, -1).join(" "))
          ? "help-group"
          : "executable-leaf",
    })),
    internalCli: [
      "delivery-run guarded by --internal-parent-run-id",
      "index --live-worker-request",
      "notify/push --internal-handoff-json and --attempt-id",
    ].map((contract) => ({
      contract,
      owners: ["T06", "T07", "T08", "T11"],
      firstProof: "V06/V07/V08 owned protocol",
      finalIntegration: "T11/V11",
    })),
    frontendInventory: "frontend-tests.json",
    runtimeSettings: keys,
    ancillaryHttp: [
      "MCP GET/POST/DELETE",
      "Swagger /docs assets",
      "/openapi.json",
      "Implicit HEAD",
      "CORS OPTIONS",
      "Static navigation/range/cache/security headers",
    ],
    rustTests: tests,
    sourceOnlyTests: sourceOnly,
    discoverySha256: digest(listingBytes),
    executionEvidence: {
      path: "output/migration/execution/v01-nextest.log",
      sha256: digest(resultBytes),
      passed: 1168,
      ignored: 13,
    },
    discoveryCaveat:
      "Framework discovery covers this target; source-only rows retain platform/example obligations. Domain overlays are conservative multi-owner partitions, not proof that every assertion is already covered by one phase.",
  };
}
