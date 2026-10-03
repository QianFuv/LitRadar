/** Export independently computed citation output and exact UTF-8 size boundaries from unchanged Rust. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

/** Run the original serializer without deriving expected text from the new implementation. */
function observe(cases) {
  const result = spawnSync(
    "output/migration/execution/citation-oracle.exe",
    [],
    {
      cwd: WORKSPACE_ROOT,
      input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
      encoding: "utf8",
      shell: false,
      windowsHide: true,
      timeout: 60_000,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const values = result.stdout
    .trim()
    .split(/\r?\n/)
    .map((line) => JSON.parse(line));
  assert.equal(values.length, cases.length);
  return values;
}

const records = [
  { title: null, authors: [], journal_title: null, date: null, doi: null },
  {
    title: "A & B <Genome>",
    authors: ["Alice", "Bob"],
    journal_title: "Journal One",
    date: "2026-01-05",
    doi: "10.1000/abc-def",
  },
  {
    title: "",
    authors: ["Author"],
    journal_title: "Journal",
    date: "2026",
    doi: "10.1000/untitled",
  },
  {
    title: "Title } { \\ % # _ & $ ~ ^\r\n@article{injected,",
    authors: ["Author\n},\n@book{injected"],
    journal_title: "Journal {unsafe}",
    date: "2026\n}",
    doi: "{}",
  },
  {
    title: "</title><record>A&B\u0000",
    authors: ["Alice O'Neil</author>"],
    journal_title: "Unused",
    date: "2026<1900",
    doi: "10.1000/a&b",
  },
  {
    title: "研究-é-🔑\u2028\u0085\u00a0\uffff",
    authors: ["\u0000\u0001\t\r\n\u007f\u009f"],
    journal_title: "Ω",
    date: "2026\rPY - 1900",
    doi: "10/中文",
  },
];
const cases = [];
for (const format of ["bibtex", "ris", "endnote"])
  for (const articles of [[], ...records.map((record) => [record]), records])
    cases.push({ format, articles, maximum_bytes: 1024 * 1024 });
const initial = observe(cases);
for (const value of initial) {
  assert(!value.limited);
  const bytes = Buffer.byteLength(value.output);
  for (const maximum_bytes of new Set([0, bytes, Math.max(0, bytes - 1)]))
    cases.push({
      format: value.format,
      articles: value.articles,
      maximum_bytes,
    });
}
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-api/src/citation.rs",
  "crates/litradar-storage/src/business/favorites.rs",
  "tests/migration/storage/citation-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
const observations = observe(cases);
await fs.writeFile(
  "tests/migration/storage/citation-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, sources, cases: observations },
    null,
    2,
  ) + "\n",
);
console.log(`Exported ${observations.length} original Rust citation cases`);
