/** Freeze settings observations from the independently compiled, unchanged Rust application. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import { BASELINE, WORKSPACE_ROOT, digest } from "../oracle.mjs";

const cases = [];
/** Add a bounded input group without deriving expected output from the Go implementation. */
function add(field, inputs) {
  for (const input of inputs) cases.push({ field, input });
}

for (const field of ["openalex_api_key_pool", "crossref_mailto_pool"])
  add(field, ["", " a; b,a\nc ", "same,SAME,same", "中文;\u0085a\u0085"]);
add("secure_cookies", [
  "",
  "True",
  " true ",
  "1",
  "YES",
  "on",
  "off",
  "NO",
  "0",
  "false",
  "2",
  "ＴＲＵＥ",
]);
for (const field of ["audit_retention_days", "delivery_worker_concurrency"])
  add(field, [
    "",
    "0",
    "+01",
    " 2 ",
    "16",
    "17",
    "3650",
    "3651",
    "4294967296",
    "1.0",
    "1e1",
    "-1",
    "++1",
  ]);
add("log_format", ["json", "compact", " json", "JSON", "compact "]);
add("cnki_captcha_token", ["", " \tsecret\n", "\u2003x\u2003"]);
for (const field of [
  "mcp_allowed_hosts",
  "cors_allowed_origins",
  "mcp_allowed_origins",
])
  add(field, [
    "",
    "localhost,localhost",
    "éxample",
    "a\tb",
    "a\nb",
    "a\u007fb",
    "null",
    "NULL",
    "https://Example.TEST:443",
    "HtTp://host:bogus",
    "http://host:",
    "http://:80",
    "http://[]",
    "http://[garbage]",
    "http://[::1]:99999",
    "https://x/",
    "https://x?",
    "https://x#",
    "https://u@x",
    "https://x%20",
    " https://host,https://host ",
  ]);
add("trusted_proxy_cidrs", [
  "",
  "192.168.2.123/24,192.168.2.1/24",
  "::ffff:192.168.0.1/128",
  "::ffff:c0a8:1/120",
  "1.2.3.4",
  "2001:db8::1/+032",
  "0.0.0.0/0,::/0",
  "127.1",
  "010.0.0.1",
  "localhost",
  "fe80::1%zone",
  "1.2.3.4/33",
  "::/129",
  "::/ 64",
  "::/++1",
]);
for (const field of ["provider_proxy_url", "ai_allowed_base_urls"])
  add(field, [
    "",
    "http://Proxy.Example",
    "https://Proxy.Example:443",
    "socks5://Proxy.Example",
    "socks5h://host/",
    "socks5://host:0",
    "https://host:0",
    "https://host:65536",
    "https://host?",
    "https://host#",
    "https://host/?",
    "https://host/#",
    "https://host/path",
    "https://host/a/../",
    "https://host/%2e/",
    "https://127.1",
    "https://0x7f000001",
    "https://0177.0.0.1",
    "https://[2001:db8::1]",
    "https://bücher.example",
    "https://xn--bcher-kva.example",
    "https://e\u0301.example",
    "https://a\u200db.example",
    "https://user:pass@host",
    "https://@host",
    "https://user@host",
    "https://:pass@host",
    "https://u:%ZZ@host",
    "https://u:%00@host",
    "HTTPS://HOST:443",
    "https:host",
    "https:///host",
    "https://host\\",
    "socks5://ÉXAMPLE",
    "socks5://u:" + "a".repeat(255) + "@host",
    "socks5://u:" + "%61".repeat(256) + "@host",
  ]);
add("ai_allowed_base_urls", [
  "https://host,https://HOST:443/",
  "https://host/a,https://host/a/,https://host/b",
]);
add("provider_proxy_policy", [
  "{}",
  '{"zjlib":false,"cnki":true}',
  '{"cnki":1,"cnki":true}',
  '{"cnki":false,"cnki":true}',
  '{"cnki":null}',
  '{"CNKI":true}',
  '{"a":false}',
  "[]",
  '{"cnki":true,"zjlib_cnki":false}',
  '{"a\\ud800":true}',
]);
add("index_provider_routes", [
  "{}",
  '{"english_journals":"zjlib_cnki"}',
  '{"cn":null,"cn":"cnki"}',
  '{"cn":"bad provider","cn":"cnki"}',
  '{"CN":"cnki"}',
  '{"cn":"CNKI"}',
]);
for (const field of [
  "article_abstract_provider_orders",
  "article_fulltext_provider_orders",
])
  add(field, [
    '{"default":[],"catalogs":{}}',
    '{"catalogs":{"cn":["zjlib_cnki"]},"default":["zjlib_cnki"]}',
    '{"default":["zjlib_cnki","zjlib"],"catalogs":{}}',
    '{"default":[],"default":[],"catalogs":{}}',
    '{"default":[],"catalogs":{},"extra":1}',
    '{"default":null,"catalogs":{}}',
    '{"default":[],"catalogs":{"cn":null,"cn":[]}}',
    '{"default":[],"catalogs":{"cn":["INVALID"],"cn":[]}}',
    '{"default":[],"catalogs":{"CN":[]}}',
    '{"default":[],"catalogs":{"cn":[1]}}',
    '{"default":["\\ud800"],"catalogs":{}}',
  ]);
add("log_filter", [
  "",
  ",,",
  " ",
  "warn",
  "WARN",
  "target",
  "6",
  "target=",
  "target=6",
  "target=+03",
  "target=info ",
  " target=info",
  "warn, ,info",
  "[",
  "[span]",
  "[]",
  "[span{}]",
  "[{field}]=debug",
  "[{=true}]=info",
  "[{field=true=ignored}]=info",
  "[span{a=1,b=2}]=info",
]);
for (const pattern of [
  "",
  "true",
  "NaN",
  "+inf",
  "1e999",
  ".*",
  "foo|bar",
  "[a-z]+",
  "[z-a]",
  "[",
  "(",
  "a{2}",
  "a{2,3}",
  "\\d+",
  "\\bword\\b",
  "(?-u:\\bword\\b)",
  "(?i)foo",
  "(?x) f o o",
  "(?R)^a$",
  "(?u)\\w",
  "(?-u)\\xff",
  "(?-u:[a-z])",
  "[a-z&&[^aeiou]]",
  "[a-z--aeiou]",
  "[a-z~~aeiou]",
  "\\pL",
  "\\p{Greek}",
  "(?<name>foo)",
  "(?P<name>foo)",
  "(?=foo)",
  "(a)\\1",
  "(?i-i)foo",
  "a**",
  "a*?",
  "a++",
  "\\a",
  "\\e",
  "\\0",
  "\\xFF",
  "\\u0041",
  "\\U00000041",
  "[[:alpha:]]",
  "[[:invalid:]]",
  "a#comment",
  "(?x)a #comment",
  "(?x)[a b]",
  "(?-u:[^a])",
])
  add("log_filter", [`[{field=${pattern}}]=info`]);

for (const pattern of [
  "(?)",
  "(?ii)",
  "(?i--m)",
  "(?i-)",
  "(?i)*",
  "()*",
  "^*",
  "[]]",
  "[a-]",
  "[--]",
  "[---a]",
  "[\\d-a]",
  "[a-\\d]",
  "(?-u:é)",
  "(?-u:\\u00FF)",
  "(?-u:[é])",
  "(?-u:[a-é])",
  "(?-u:.)",
  "(?-u:\\D)",
  "(?-u:[\\D&&a])",
  "(?-u:[\\D--\\D])",
  "(?-u:[[:^ascii:]&&a])",
  "(?-u:[[^a]&&a])",
  "(?i-u:[A--a])",
  "(?i-u:[A&&a])",
  "(?-u:\\<a\\>)",
  "\\<a\\>",
  "\\é",
  "\\!",
  "(?<α>foo)",
  "(?<a.b>foo)",
  "(?<a>f)(?<a>g)",
  "(?x) a #text\nb",
  "(?x)[ a #text\nb ]",
  "(?x)\\ ",
  "(?x)\\#",
  "[a&&]",
  "[&&a]",
  "[a~~]",
  "[a--]",
  "[a&&b--c]",
  "[]",
  "[^^]",
  "\\pX",
])
  add("log_filter", [`[{field=${pattern}}]=info`]);
for (const depth of [249, 250, 251])
  for (const pattern of [
    "(".repeat(depth) + "a" + ")".repeat(depth),
    "a" + "*".repeat(depth),
    "[".repeat(depth) + "a" + "]".repeat(depth),
    "[".repeat(depth) + "ab" + "]".repeat(depth),
  ])
    add("log_filter", [`[{field=${pattern}}]=info`]);
const classBodies = [
  "a",
  "A",
  "a-z",
  "^a",
  "\\d",
  "\\D",
  "[:alpha:]",
  "[:^ascii:]",
  "\\x7f",
  "\\x80",
  "é",
  "[a-z]",
  "[^a]",
];
for (const left of classBodies)
  for (const right of ["a", "[a-z]", "\\D"])
    for (const operator of ["", "&&", "--", "~~"])
      add("log_filter", [`[{field=(?i-u:[${left}${operator}${right}])}]=info`]);

let seed = 0x517e03;
const atoms = [
  "a",
  "b",
  "é",
  "|",
  "(",
  ")",
  "*",
  "+",
  "?",
  "^",
  "$",
  ".",
  "[",
  "]",
  "-",
  "&&",
  "\\d",
  "\\D",
  "\\b",
  "\\<",
  "(?i)",
  "(?-u)",
  "(?x)",
  "(?:",
  "\\!",
  "#",
  " ",
];
for (let index = 0; index < 1000; index++) {
  let pattern = "";
  for (let part = 0; part < 1 + (index % 8); part++) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    pattern += atoms[seed % atoms.length];
  }
  add("log_filter", [`[{field=${pattern}}]=info`]);
}

add("log_filter", ["\u0345=info"]);
for (const pattern of [
  "(?P<\u0345>a)",
  "(?P<\u2160>a)",
  "\\pl",
  "\\Pn",
  "(?x)\\x 4 1",
  "(?x)\\u 0 0 4 1",
  "(?x)\\U 0 0 0 0 0 0 4 1",
  "(?x)\\p L",
  "(?x)( ?i:a)",
  "(?x)[a- ]",
  "(?x)a" + "* ?".repeat(125),
  "+NaN",
  "-NaN",
  "NAN",
  "+infinity",
])
  add("log_filter", [`[{field=${pattern}}]=info`]);

for (const pattern of [
  "(?x)[a-#x\n]",
  "(?x)[a-#\n]",
  "(?x)[a-# x\n]",
  "(?x)[a- #x\nb]",
  "+-NaN",
])
  add("log_filter", [`[{field=${pattern}}]=info`]);

const result = spawnSync("output/migration/execution/settings-oracle.exe", [], {
  cwd: WORKSPACE_ROOT,
  input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
  encoding: "utf8",
  timeout: 60_000,
  maxBuffer: 8 * 1024 * 1024,
  shell: false,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
const sources = [];
for (const filename of [
  "Cargo.lock",
  "crates/litradar-storage/src/business/runtime_settings.rs",
  "tests/migration/auth/settings-oracle.rs",
  "target/debug/liblitradar_storage.rlib",
  "target/debug/deps/libserde_json-aef12c909ad483bc.rlib",
])
  sources.push({ path: filename, sha256: digest(await fs.readFile(filename)) });
await fs.writeFile(
  "tests/migration/auth/settings-vectors.json",
  JSON.stringify(
    { baseline: BASELINE, sources, cases: observations },
    null,
    2,
  ) + "\n",
);
console.log(
  `Exported ${observations.length} independent Rust settings observations`,
);
