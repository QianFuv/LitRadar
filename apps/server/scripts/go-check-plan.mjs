/** Define complete Go verification lanes without executing native builds or tests. */
import assert from "node:assert/strict";

export const SERVER_PACKAGES = [".", "./apps/server/..."];

/**
 * Enable cgo only for race-detector commands, which require it; every other check builds pure Go.
 * @param {string[]} argumentsList - Check command arguments.
 * @returns {"0" | "1"} CGO_ENABLED value for the check.
 */
export function cgoSetting(argumentsList) {
  return argumentsList.includes("-race") ? "1" : "0";
}

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
      ["format", "node", ["apps/server/scripts/check-go-format.mjs"]],
      ["modules", "go", ["mod", "verify"]],
      [
        "sdk",
        "node",
        ["apps/server/scripts/verify-go-dependency.mjs", "go-sdk"],
      ],
      ["vet", "go", ["vet", "-mod=readonly", ...SERVER_PACKAGES]],
    );
  }
  for (const mode of ["regular", "race"]) {
    if (lane !== "all" && lane !== mode) continue;
    const flags = [
      "test",
      "-count=1",
      "-json",
      "-mod=readonly",
      ...(mode === "race" ? ["-race"] : []),
    ];
    checks.push([
      mode,
      "go",
      [
        ...flags,
        "-timeout=20m",
        ...(mode === "regular" ? ["-vet=off"] : []),
        ...SERVER_PACKAGES,
      ],
    ]);
    checks.push([
      `sdk-own-${mode}`,
      "go",
      ["-C", "third_party/go-sdk", ...flags, "./mcp"],
    ]);
    checks.push([
      `sdk-root-${mode}`,
      "go",
      [...flags, "github.com/modelcontextprotocol/go-sdk/mcp"],
    ]);
  }
  return checks;
}
