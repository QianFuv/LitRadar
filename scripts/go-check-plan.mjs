/** Define complete Go verification lanes without executing native builds or tests. */
import assert from "node:assert/strict";

const TAGS = "sqlite_fts5,sqlite_dbstat";

/**
 * Select independent regular/race lanes while retaining the complete local default.
 * @param {string} lane - all, regular, or race.
 * @returns {Array<[string, string, string[]]>} Ordered checks with stable diagnostic IDs.
 */
export function goCheckPlan(lane = "all") {
  assert(["all", "regular", "race"].includes(lane), "Unknown Go check lane");
  const checks = [["environment", "go", ["env", "-json"]]];
  if (lane !== "race") {
    checks.push(
      ["format", "node", ["scripts/check-go-format.mjs"]],
      ["modules", "go", ["mod", "verify"]],
      ["sdk", "node", ["scripts/verify-go-dependency.mjs", "go-sdk"]],
      ["sqlite", "node", ["scripts/verify-go-dependency.mjs", "go-sqlite3"]],
      ["vet", "go", ["vet", "-mod=readonly", "-tags", TAGS, "./..."]],
    );
  }
  for (const mode of ["regular", "race"]) {
    if (lane !== "all" && lane !== mode) continue;
    const flags = [
      "test",
      "-count=1",
      "-json",
      "-mod=readonly",
      "-tags",
      TAGS,
      ...(mode === "race" ? ["-race"] : []),
    ];
    checks.push([
      mode,
      "go",
      [
        ...flags,
        "-timeout=20m",
        ...(mode === "regular" ? ["-vet=off"] : []),
        "./...",
      ],
    ]);
    for (const [name, module, packageName] of [
      ["sdk", "go-sdk", "github.com/modelcontextprotocol/go-sdk/mcp"],
      ["sqlite", "go-sqlite3", "github.com/mattn/go-sqlite3"],
    ]) {
      checks.push([
        `${name}-own-${mode}`,
        "go",
        [
          "-C",
          `third_party/${module}`,
          ...flags,
          name === "sdk" ? "./mcp" : "./...",
        ],
      ]);
      checks.push([`${name}-root-${mode}`, "go", [...flags, packageName]]);
    }
  }
  return checks;
}
