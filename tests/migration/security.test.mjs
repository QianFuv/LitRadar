/** Reject incomplete release inventories, including embedded data and patched dependencies. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { verifyImageSources } from "./security.mjs";

for (const roots of [["third_party"], ["cmd", "internal", "assets"]])
  test("image inventory must be complete: " + roots.join(","), () => {
    const original = process.cwd();
    const directory = fs.mkdtempSync(
      path.join(os.tmpdir(), "image-inventory-"),
    );
    try {
      process.chdir(directory);
      const files = [
        "cmd/main.go",
        "internal/tools.json",
        "internal/README.md",
        "assets/catalog.json",
        "third_party/driver.go",
        "third_party/LICENSE",
      ];
      for (const file of files) {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, file);
      }

      const selected = files.filter((file) =>
        roots.includes(file.split("/")[0]),
      );
      const lines = selected.map(
        (file) => createHash("sha256").update(file).digest("hex") + "  " + file,
      );
      assert.doesNotThrow(() => verifyImageSources(lines.join("\n"), roots));
      const omitted =
        roots[0] === "third_party"
          ? "third_party/driver.go"
          : "internal/tools.json";
      assert.throws(
        () =>
          verifyImageSources(
            lines.filter((line) => !line.endsWith("  " + omitted)).join("\n"),
            roots,
          ),
        /Image source file set differs/,
      );
      assert.throws(
        () => verifyImageSources([...lines, lines[0]].join("\n"), roots),
        /Image source file set differs/,
      );
      assert.throws(
        () =>
          verifyImageSources(
            lines.join("\n").replace(/^[a-f0-9]{64}/, "0".repeat(64)),
            roots,
          ),
        /Image source differs/,
      );
    } finally {
      process.chdir(original);
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
