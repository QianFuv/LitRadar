/** Freeze domestic HTML behavior from original public Rust parsers and original fixture bodies. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const sourcePath = "crates/litradar-sources/src/cnki_domestic.rs";
const source = await fs.readFile(sourcePath, "utf8");
const fixtures = {};
for (const name of ["SEARCH", "DETAIL", "YEAR", "PAPERS", "ABSTRACT"]) {
  const match = source.match(
    new RegExp(`const ${name}_HTML: &str = r#"([\\s\\S]*?)"#;`),
  );
  assert.ok(match, `Missing original ${name} fixture`);
  fixtures[name] = match[1];
}
const cases = [];
const url = "https://kns.cnki.net/kcms2/article/abstract?v=test";
/** Register an independently evaluated parser input. */
function add(id, kind, text, extra = {}) {
  cases.push({
    id,
    kind,
    text,
    url,
    issue: { year: 2025, number: "12" },
    page: 0,
    ...extra,
  });
}
for (const [kind, name] of [
  ["search", "SEARCH"],
  ["detail", "DETAIL"],
  ["years", "YEAR"],
  ["papers", "PAPERS"],
  ["article", "ABSTRACT"],
]) {
  add(`${kind}-original`, kind, fixtures[name]);
  for (const text of [
    "",
    " \n\t",
    "<html></html>",
    "captcha",
    "安全验证",
    '"code":-403',
    "访问异常",
    "/verify/home",
    "暂无数据",
    "no results",
    "文献不存在",
    "record has been deleted",
    "该刊数据正在更新中，请耐心等待",
  ]) {
    add(`${kind}-marker-${cases.length}`, kind, text);
    add(`${kind}-content-marker-${cases.length}`, kind, fixtures[name] + text);
  }
  for (const transform of [
    (text) => text.replaceAll('"', "'"),
    (text) => text.replaceAll("<input", "<INPUT"),
    (text) => text.replaceAll("<a", "<A"),
    (text) => text.replaceAll('="', '= "'),
    (text) => text.replaceAll("\n", ""),
    (text) => text.replaceAll("&amp;", "&amp;amp;"),
  ]) {
    add(`${kind}-syntax-${cases.length}`, kind, transform(fixtures[name]));
  }
}
for (const fragment of [
  '<h3 class="author" id="authorpart"><span><a>张<span>三</span></a><sup><span>1,2</span></sup></span><span>李四<sup>1，4*</sup></span><span><sup>3</sup></span><span>Team 2 &amp; Henry VIII</span></h3>',
  '<h3 class="author" id="authorpart"><span> A\n B <script>evil</script><style>bad</style><sup>1</sup></span></h3>',
  '<h3 class="author" id="authorpart"><div><span>A</span></div><span>B</span></h3>',
  '<h3 class="author" id="authorpart">A; B</h3><span class="author">Fallback</span>',
  '<h3 class="author" id="authorpart"><span><sup>1</sup></span></h3><h3 class="author" id="authorpart"><span>B</span></h3>',
  '<span class="author" title="  A &#x26; B  ">fallback</span>',
  '<SPAN CLASS="author" TITLE="A &copy; B">fallback</SPAN>',
  '<span class="author" title=""> A <sup>1</sup>B </span>',
  '<span class="author" title="&amp;nbsp;A">fallback</span>',
  '<span class="author">A&nbsp; B</span>',
  '<h3 class="author" id="authorpart"><template><span>T</span></template><span>B</span></h3>',
  '<table><h3 class="author" id="authorpart"><span>A</span></h3></table>',
  '<h3 class="author" id="authorpart"><svg><span>A</span></svg><span>B</span></h3>',
])
  add(`authors-${cases.length}`, "article", fixtures.ABSTRACT + fragment);
for (const value of [
  url,
  "/kcms2/article/abstract?v=%2B+%26&empty&uniplatform=OTHER&Language=EN#part",
  "/article/abstract?v=%FF%FE%ED%A0%80",
  "/starter?a=1",
  "/verify/home",
  "/knavi/detail?p=a",
  "//kns.cnki.net/kcms2/article/abstract",
  "http://kns.cnki.net/",
  "https://oversea.cnki.net/",
  "https://kns.cnki.net:443/a",
  "https://kns.cnki.net:444/a",
  "https://user:pw@kns.cnki.net/",
  "https://@kns.cnki.net/",
  "https://KNS.CNKI.NET/a?x=%bad",
  "https:kns.cnki.net/a",
  "relative",
  "",
  "https://kns.cnki.net/a?x=1&&bare&x=2&%75niplatform=X",
  "https://kns.cnki.net/a?x=%00%0A&language&LANGUAGE=EN",
  "https://kns.cnki.net/a?#",
  "https://kns.cnki.net/a?x=~!*'()",
])
  add(`article-url-${cases.length}`, "article", fixtures.ABSTRACT, {
    url: value,
  });
for (const count of [
  "0",
  "1",
  "2",
  "10",
  "11",
  "+1",
  "01",
  "-1",
  "1.0",
  " 1 ",
  "18446744073709551615",
  "18446744073709551616",
]) {
  add(
    `papers-count-${count}`,
    "papers",
    fixtures.PAPERS.replace('value="1"', `value="${count}"`),
  );
}
const row = fixtures.PAPERS.match(/<dd[\s\S]*?<\/dd>/)[0];
for (const count of [0, 1, 9, 10, 11])
  add(
    `papers-rows-${count}`,
    "papers",
    `<dt>Section</dt>${row.repeat(count)}<input id="articleCount" value="${count}">`,
  );
for (const page of [0, 1, 2])
  for (const text of [
    "该刊数据正在更新中，请耐心等待",
    "暂无相关数据",
    '<input id="articleCount" value="0">',
    "captcha 暂无数据",
  ])
    add(`papers-empty-${cases.length}`, "papers", text, { page });
for (const key of [
  "202500",
  "2025",
  "2025abc",
  "20250012",
  "+12301",
  "-12301",
  "二〇二五",
  "202",
  "2025🧪",
  "000000",
])
  add(
    `year-${key}`,
    "years",
    `YearIssueTree<a id="yq${key}" value="&amp;amp;token">No. 00012</a>`,
  );
for (const fragment of [
  '<input id="paramfilename" value="">',
  '<h1 class="title"> Main &amp; <i>Title</i></h1>',
  '<p class="title-one">Alternate</p>',
  '<span class="rowtit">DOI：</span><span>first</span><p>later</p>',
  '<span class="rowtit">DOI：</span><p></p><span>next</span>',
  '<span class="rowtit">在线公开时间：</span><p>2026-10-01 12:00:00</p> Pages: 12-30；end',
  '<span id="ChDivSummary">A &copy; B &lt;x&gt;</span>',
  '<input id="abstract_text" value="&amp;lt;x&amp;gt;A">',
])
  add(
    `article-fields-${cases.length}`,
    "article",
    fragment + fixtures.ABSTRACT,
  );
for (const keyword of ["世界经济", "  A & B  ", "<>&\u2028", 'a"\\\n'])
  for (const field of ["TI", "SN", ""])
    add(`form-${cases.length}`, "form", keyword, { field });
add("locator", "locator", "", {
  titles: [
    " A  B ",
    "a b",
    "Ａ Ｂ",
    "世界（经济）",
    "世界(经济)",
    "e\u0301",
    "é",
    "",
  ],
  issns: ["1002-9621", "10029621", "0000-0000", "1002-962X", "", "2049-3630"],
});
for (const fragment of [
  '<h3 class="author" id="authorpart"><span>A<template><span>B</span></template>C</span></h3>',
  '<table><tr><td><span class="author">A</span></td></tr><span class="author">B</span></table>',
])
  add(
    `authors-review-${cases.length}`,
    "article",
    fixtures.ABSTRACT + fragment,
  );
for (const text of [
  "",
  "captcha",
  "安全验证",
  "captcha 暂无数据",
  "oversea.cnki.net captcha",
  "/verify/home?captchaId=id&amp;ident=i",
  'location="https://kns.cnki.net/verify/home?captchaId=id"',
  JSON.stringify({ message: "/verify/home?captchaId=id" }),
  JSON.stringify({ message: "https://evil.test/verify/home?a=b" }),
]) {
  for (const challenge of [
    url,
    "https://kns.cnki.net/verify/home?captchaId=id",
    "https://navi.cnki.net/verify/home",
    "https://kns.cnki.net/a?return=/verify/home",
    "https://kns.cnki.net/VERIFY/HOME",
  ])
    add(`challenge-${cases.length}`, "challenge", text, { url: challenge });
}
const puzzle = {
  originalImageBase64: "data:image/png;base64, ORIGINAL ",
  jigsawImageBase64: "JIGSAW",
  secretKey: "0123456789abcdef",
  token: "token",
  captchaId: "body-id",
};
for (const body of [
  puzzle,
  { repData: puzzle },
  { data: puzzle },
  { repData: { a: puzzle } },
  { data: { z: puzzle, a: { ...puzzle, token: "first" } } },
  { z: puzzle },
  { repData: null, data: puzzle },
  { originalImageBase64: null, repData: puzzle },
  [],
  null,
  {},
  ...Object.keys(puzzle).map((key) => ({ ...puzzle, [key]: null })),
  ...Object.keys(puzzle).map((key) => ({ ...puzzle, [key]: "" })),
  { ...puzzle, secretKey: "汉字汉字ab" },
]) {
  for (const challenge of [
    "/verify/home",
    "/verify/home?captchaId=query-id&captchaType=&ident=a%2Bb+c&returnUrl=%2F",
    "/verify/home?captchaId=",
    "/verify/home?captchaId=first&captchaId=last&captchaType=x",
    "https://navi.cnki.net/verify/home",
  ])
    add(`puzzle-${cases.length}`, "puzzle", "", { url: challenge, body });
}
for (const value of [true, false, 1, 0, 1.5, "true", "TRUE", "1", null, {}, []])
  for (const body of [
    { success: value },
    { repData: { result: value } },
    { data: { result: value } },
    { repData: null, data: { result: value } },
    { repData: { result: false }, data: { result: value } },
  ])
    add(`success-${cases.length}`, "success", "", { body });
for (const tag of ["title", "style", "script", "textarea", "xmp"])
  add(
    `authors-foreign-${tag}`,
    "article",
    fixtures.ABSTRACT +
      `<svg><${tag}><span class="author">A</span></${tag}></svg><span class="author">B</span>`,
  );
for (const fragment of [
  '<table><tr><td><h3 class="author" id="authorpart"><span>A</span></h3></td></tr><h3 class="author" id="authorpart"><span>B</span></h3></table>',
  '<script>let broken="<span attr=\'";</script><span class="author">A</span><span class="author">B</span>',
  '<svg><![CDATA[<span class="author">fake</span>]]></svg><span class="author">A</span>',
  '<span data-litradar-source-order="0" class=author>A</span><span class=author>B</span>',
  '<h3 class="author" id="authorpart"><table><tr><td><span>A</span></td></tr><span>B</span></table></h3>',
])
  add(
    `authors-structure-${cases.length}`,
    "article",
    fixtures.ABSTRACT + fragment,
  );
const fixtureData = {
  journal_search_html: fixtures.SEARCH,
  journal_detail_html: fixtures.DETAIL,
  year_issues_html: fixtures.YEAR,
  issue_article_pages: {
    202512: [fixtures.PAPERS, '<input id="articleCount" value="0">'],
  },
  article_detail_html: { id: fixtures.ABSTRACT, "": fixtures.ABSTRACT },
  article_detail_status_codes: {},
};
const operations = [
  { op: "resolve", titles: ["世界经济"], issns: ["1002-9621"] },
  { op: "resolve", titles: ["世界经济"], issns: ["2049-3630"] },
  { op: "resolve", titles: ["世界经济"], issns: [] },
  { op: "years", journal: {} },
  {
    op: "papers",
    issue: { year_issue_id: "202512", year: 2025, number: "12" },
    page: 0,
  },
  { op: "papers", issue: { year_issue: "202512" }, page: 1 },
  { op: "papers", issue: { year_issue_id: "202512" }, page: 2 },
  { op: "papers", issue: { year_issue_id: 12 }, page: 0 },
  { op: "article", url, platform_id: "id" },
  { op: "article", url, platform_id: "" },
  { op: "article", url, platform_id: null },
  { op: "reset" },
  { op: "drain" },
  { op: "drain" },
];
for (const fixture of [
  fixtureData,
  {},
  ...["journal_detail", "year_issues", "issue_articles", "article_detail"].map(
    (fail_endpoint) => ({ ...fixtureData, fail_endpoint }),
  ),
  ...[0, 200, 404, 410, 429, 500, 65535].map((status) => ({
    ...fixtureData,
    article_detail_status_codes: { id: status },
    fail_endpoint: "article_detail",
  })),
  { ...fixtureData, journal_detail_html: "captcha" },
  { ...fixtureData, year_issues_html: "captcha" },
  { ...fixtureData, article_detail_html: { id: "record does not exist" } },
])
  add(`fixture-${cases.length}`, "fixture", "", { fixture, operations });
for (const text of [
  "{}",
  "[]",
  '["search"]',
  '{"unknown":"\\uD800"}',
  '{"unknown":1e999}',
  '{"journal_detail_html":"a","journal_detail_html":"b"}',
  ...Object.keys(fixtureData).flatMap((key) => [
    JSON.stringify({ [key]: null }),
    JSON.stringify({ [key]: 0 }),
  ]),
  '{"issue_article_pages":{"a":null}}',
  '{"issue_article_pages":{"a":[null]}}',
  '{"article_detail_html":{"a":null}}',
  '{"article_detail_status_codes":{"a":65536}}',
  '{"article_detail_status_codes":{"a":1.0}}',
  '{"fail_endpoint":null}',
  '{"fail_endpoint":"article_detail"}',
])
  add(`fixture-decode-${cases.length}`, "fixture_decode", text);
for (const budget of [0, 1, 5])
  for (const distance of [0, 0.5, 100.4, 9999.9, -1])
    for (const failures of [0, 1])
      add(`solve-${cases.length}`, "solve", "", {
        budget,
        distance,
        failures,
        attach: [
          url + "&x=%2B+~",
          url + "&",
          url + "&captchaId=old",
          url + "&%63aptchaId=old",
          url + "&CAPTCHAID=old",
          "http://kns.cnki.net/",
        ],
        operations: [
          {
            text: "captcha",
            url: "/verify/home?captchaId=first",
            body: puzzle,
            accept_after: 3,
          },
          {
            text: "captcha",
            url: "/verify/home?captchaId=second",
            body: puzzle,
            accept_after: 1,
          },
          { text: "YearIssueTree captcha", url, body: puzzle },
          {
            text: "captcha",
            url: "/verify/home?captchaId=third",
            body: puzzle,
            fail: "fetch",
          },
        ],
      });
add(
  "authors-decoded-marker-collision",
  "article",
  fixtures.ABSTRACT +
    '<p> data-litradar-source-orde&#114;="1"</p><table><tr><td><span class="author">A</span></td></tr><span class="author">B</span></table>',
);
add("challenge-empty-url-path", "challenge", "x", { url: "?/verify/home" });
for (const [kind, object] of [
  ["anchor", { version: 1, year_issue_id: "202501" }],
  [
    "checkpoint",
    {
      version: 2,
      base_anchor_issue_id: null,
      candidate_head_issue_id: "202501",
      current_issue_id: "202412",
      page_index: 0,
    },
  ],
]) {
  const keys = Object.keys(object);
  const inputs = [
    JSON.stringify(object),
    JSON.stringify(Object.values(object)),
    "{}",
    "[]",
    "null",
    JSON.stringify({ ...object, unknown: 1 }),
    ...keys.map((key) =>
      JSON.stringify(
        Object.fromEntries(
          Object.entries(object).filter(([name]) => name !== key),
        ),
      ),
    ),
    ...keys.flatMap((key) =>
      [null, true, 1.0, 1.5, "", [], {}, "<>&\u2028"].map((value) =>
        JSON.stringify({ ...object, [key]: value }),
      ),
    ),
  ];
  for (const token of [
    "-0",
    "-1",
    "4294967295",
    "4294967296",
    "18446744073709551615",
    "18446744073709551616",
    "1e0",
    "1.0",
  ]) {
    inputs.push(
      JSON.stringify(object).replace(
        '"version":' + object.version,
        '"version":' + token,
      ),
    );
    if (kind === "checkpoint")
      inputs.push(
        JSON.stringify(object).replace(
          '"page_index":0',
          '"page_index":' + token,
        ),
      );
  }
  inputs.push(JSON.stringify(object).replace(/}$/, ',"version":1}'));
  for (const text of inputs) add(`${kind}-decode-${cases.length}`, kind, text);
}
const result = spawnSync(
  "output/migration/execution/sources-cnki-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/cnki-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-cnki-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-cnki.mjs"),
      ),
      original_fixtures: {
        path: sourcePath,
        sha256: digest(Buffer.from(source)),
      },
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust CNKI observations`);
