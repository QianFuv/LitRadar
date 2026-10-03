/** Export a deterministic malformed-input corpus using the separately compiled Rust oracle. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const corpus = new Set();
for (const value of [
  "",
  " \r\n\t",
  "{}x",
  "[]",
  '{"a":1}',
  '[true,false,null,"中文",-1.2e+3]',
  '{"jsonrpc":"2.0","id":1,"method":"ping"}',
  '{"jsonrpc":"2.0","id":9223372036854775807,"method":"ping"}x',
  "1.",
  "1e",
  "1e+",
  "1e-",
  "1.e",
  "01",
  "-01",
  "-x",
  "1e999",
  "1e21474836480",
  "0e21474836480",
  "-1e-21474836480",
  "1e999 ",
  "1e999]",
  "1e+X",
  '"\\uX000"',
  '"\\uX',
  '"\\ud800"',
  '"\\udc00"',
  '"\\ud800\\u0041"',
  '"\\ud800\\udc00"',
  '"\\ud800\\x"',
  '"\\u001g"',
  '"\\uXYZZ"',
  '"a\nb"',
  '"a\rb"',
  '"a\\x"',
  '{"a":1,}',
  "[1,]",
  '{"a" 1}',
  '{"a":}',
  "[1 2]",
  "{1:2}",
  "trueX",
  "falSe",
  "nulL",
]) {
  const bytes = Buffer.from(value);
  for (let length = 0; length <= bytes.length; length++)
    corpus.add(bytes.subarray(0, length).toString("hex"));
}
for (const middle of [
  [0xff],
  [0xc0, 0x80],
  [0xed, 0xa0, 0x80],
  [0xf4, 0x90, 0x80, 0x80],
  [0xef, 0xbf, 0xbd],
]) {
  for (const tail of [
    [],
    [34],
    [10, 34],
    [92, 120, 34],
    [92, 117, 48, 48, 52, 49, 34],
  ])
    corpus.add(Buffer.from([34, ...middle, ...tail]).toString("hex"));
}
for (const depth of [126, 127, 128, 129])
  for (const [left, right] of [
    ["[", "]"],
    ['{"a":', "}"],
  ])
    corpus.add(
      Buffer.from(left.repeat(depth) + "0" + right.repeat(depth)).toString(
        "hex",
      ),
    );
const result = spawnSync("output/migration/execution/json-oracle.exe", [], {
  cwd: WORKSPACE_ROOT,
  input: [...corpus].join("\n") + "\n",
  encoding: "utf8",
  timeout: 30_000,
  maxBuffer: 4 * 1024 * 1024,
  shell: false,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const cases = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(cases.length, corpus.size);
const sources = [];
for (const path of [
  "tests/migration/primitives/json-oracle.rs",
  "target/debug/deps/libserde_json-4b04879bb0b79b6d.rlib",
  "target/debug/deps/librmcp-066ed2a76f20a06d.rlib",
])
  sources.push({ path, sha256: digest(await fs.readFile(path)) });
await fs.writeFile(
  "tests/data/migration/json-syntax.json",
  JSON.stringify(
    {
      baseline: BASELINE,
      serdeJson: "1.0.150",
      rmcp: "2.1.0",
      reader: "from_reader",
      sources,
      cases,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Exported ${cases.length} independent Rust syntax/envelope observations`,
);
