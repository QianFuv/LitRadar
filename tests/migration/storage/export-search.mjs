/** Freeze original Rust Unicode scalar projections, FTS grammar and strict author shapes. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Observe one process with explicit resource bounds and unmodified original application dependencies. */
function observe(requests) {
  const result = spawnSync("output/migration/execution/search-oracle.exe", [], {
    cwd: WORKSPACE_ROOT,
    input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 64 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout
    .trim()
    .split(/\r?\n/)
    .map((line) => JSON.parse(line));
}

const scalar_blocks = observe([{ kind: "scalars" }]).map(
  ({ start, values }) => {
    const hash = createHash("sha256");
    for (const value of values) {
      const bytes = Buffer.from(value);
      const size = Buffer.alloc(4);
      size.writeUInt32LE(bytes.length);
      hash.update(size);
      hash.update(bytes);
    }
    return { start, count: values.length, sha256: hash.digest("hex") };
  },
);
const requests = [];
const inputs = [
  "",
  "ASCII gene NOT preview",
  "Café AND résumé OR ΣΙΓΜΑ NOT 科技金融",
  "ÀND",
  "gene ÓR cancer",
  "title:Café* NOT preview",
  '"títle":gene',
  "{títle abstract_text}:Café",
  "\u0301",
  'Café "unterminated',
  '"Café""résumé"',
  '"x"\v:Ω',
  "\u001a_title:ABC",
  "\u1c89",
  "title:\u1c89*",
  "\u1c89:gene",
  "\u{105c9}",
  "\u0130ΣΟΣ",
  "\u00a0\u0085\u2028",
  "\u000b",
  "a\u0301\u0327",
  "\ue000\u{f0000}\u{10fffd}",
];
for (const input of inputs)
  for (const uses_simple of [false, true])
    for (const mode of ["simple", "advanced"])
      requests.push({ kind: "text", input, uses_simple, mode });
for (const input of [
  "[]",
  '["A","B"]',
  '[{"display_name":"A"}]',
  '[["A"]]',
  '[["A"],{"display_name":"B"}]',
  '["A",{"display_name":"B"}]',
  '[{"display_name":"A","extra":1}]',
  '[{"display_name":"A","display_name":"B"}]',
  '[{"display_name":null}]',
  "[null]",
  "null",
  "{}",
  "[1]",
  '[["A","B"]]',
  '[{"display_name":"\\ud800"}]',
  '["\\ud800"]',
])
  requests.push({ kind: "authors", input });
const cases = observe(requests);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/search_text.rs",
  "crates/litradar-storage/src/article_authors.rs",
  "tests/migration/storage/search-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/storage/search-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, sources, scalar_blocks, cases },
    null,
    2,
  ) + "\n",
);
console.log(
  `Exported ${scalar_blocks.reduce((sum, block) => sum + block.count, 0)} Unicode scalars and ${cases.length} search/author observations`,
);
