/** Preserve the original field matcher's Unicode tables independently of the Go runtime. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";

const root = process.argv[2];
assert(root, "provide the locked regex-syntax-0.8.11 source directory");
const manifest = await fs.readFile(path.join(root, "Cargo.toml"), "utf8");
assert.match(manifest, /name = "regex-syntax"/);
assert.match(manifest, /version = "0\.8\.11"/);
const output = "internal/runtime/logfilter/unicode16";
await fs.mkdir(output, { recursive: true });
const sources = [],
  tables = {},
  folds = {};
const characterPattern = /'((?:\\.|[^'\\])*)'/gu;

/** Decode one Rust Unicode scalar without evaluating source code. */
function scalar(value) {
  let decoded = value;
  if (value.startsWith("\\u{"))
    decoded = String.fromCodePoint(Number.parseInt(value.slice(3, -1), 16));
  else if (value.startsWith("\\"))
    decoded = {
      "\\n": "\n",
      "\\r": "\r",
      "\\t": "\t",
      "\\0": "\0",
      "\\\\": "\\",
      "\\'": "'",
    }[value];
  assert.equal([...decoded].length, 1, `Invalid Rust scalar ${value}`);
  return decoded.codePointAt(0);
}
for (const [filename, selected] of [
  [
    "general_category",
    {
      LETTER: "L",
      MARK: "M",
      NUMBER: "N",
      PUNCTUATION: "P",
      SYMBOL: "S",
      SEPARATOR: "Z",
      OTHER: "C",
    },
  ],
  ["perl_word", { PERL_WORD: "w" }],
  ["perl_space", { WHITE_SPACE: "s" }],
  ["perl_decimal", { DECIMAL_NUMBER: "d" }],
  ["case_folding_simple", {}],
]) {
  const bytes = await fs.readFile(
    path.join(root, "src/unicode_tables", filename + ".rs"),
  );
  const text = bytes.toString("utf8");
  assert.match(text, /Unicode version: 16\.0\.0/);
  sources.push({ file: filename + ".rs", sha256: digest(bytes) });
  if (filename === "case_folding_simple") {
    for (const line of text
      .split(/\r?\n/)
      .filter((line) => line.trimStart().startsWith("("))) {
      const values = [...line.matchAll(characterPattern)].map((match) =>
        scalar(match[1]),
      );
      assert(values.length >= 2);
      folds[values[0]] = values.slice(1);
    }
    continue;
  }
  for (const [name, key] of Object.entries(selected)) {
    const start = text.indexOf(`pub const ${name}:`);
    assert(start >= 0, `${filename} ${name}`);
    const content = text.slice(
      text.indexOf("=", start) + 1,
      text.indexOf("];", start),
    );
    const values = [...content.matchAll(characterPattern)].map((match) =>
      scalar(match[1]),
    );
    assert(values.length > 0 && values.length % 2 === 0);
    tables[key] = values;
  }
}
const content = JSON.stringify({ tables, folds }) + "\n";
await fs.writeFile(path.join(output, "tables.json"), content);
for (const name of ["LICENSE-MIT", "LICENSE-APACHE"])
  await fs.copyFile(path.join(root, name), path.join(output, name));
await fs.copyFile(
  path.join(root, "src/unicode_tables/LICENSE-UNICODE"),
  path.join(output, "LICENSE-UNICODE"),
);
await fs.writeFile(
  path.join(output, "README.md"),
  `# Original logging Unicode tables\n\nUnicode 16.0.0 ranges and simple case folding from locked regex-syntax 0.8.11. Embedded as compatibility data; no runtime dependency. This prevents changes to Go's Unicode release from changing persisted log-filter semantics.\n\nUpstream: https://github.com/rust-lang/regex/tree/regex-syntax-0.8.11/regex-syntax/src/unicode_tables\n\nGenerated tables.json SHA-256: ${digest(Buffer.from(content))}\n\nRegenerate: node tests/migration/runtime/export-log-unicode.mjs PATH_TO_REGEX_SYNTAX_0.8.11\n\nSource hashes:\n\n${sources.map((source) => `- ${source.file}: ${source.sha256}`).join("\n")}\n`,
);
console.log(
  JSON.stringify({
    tables: Object.keys(tables),
    folds: Object.keys(folds).length,
    sha256: digest(Buffer.from(content)),
  }),
);
