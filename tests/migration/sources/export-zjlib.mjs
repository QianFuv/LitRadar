/** Freeze original library helpers and session transitions with a deterministic clock. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const cases = [];
/** Append one independent observation. */
function add(kind, extra) {
  cases.push({ id: `${kind}-${cases.length}`, kind, ...extra });
}
for (const text of [
  "",
  " \n\t",
  "&amp;lt;&lt;x&gt;&nbsp;&#160;",
  "&#xD800;&#1114112;&#0;&#x+41;&#++65;",
  "A<B>C</B>D>",
  "Ada Lovelace；Grace Hopper",
  "İ ΟΣ ΟΣΑ ẞ Ⅷ ² \u0345 \u2163",
  '... /\\:*?"<>| ...',
  "中文".repeat(80),
  " ".repeat(130) + "end",
  "a".repeat(119) + "😀界",
  "&amp;amp; &AMP; &unknown;",
  "e\u0301 é",
  "作者、作者，姓 名",
])
  add("text", { text });
for (const entity of [
  "#",
  "#x",
  "#X41",
  "#x+41",
  "#+65",
  "#-1",
  "#4294967296",
  "#x10ffff",
  "#xdfff",
  "quot",
  "apos",
  "amp",
  "nbsp",
  "#009",
  "# 65",
  "#0x41",
])
  add("text", { text: `before &${entity}; after` });
for (const text of [
  "success",
  "live_success",
  "start_failure",
  "timeout",
  "poll_timeout",
  "poll_failure",
  "warmup_failure",
  "fulltext_mismatch",
  "fulltext_failure",
  " Success ",
  " success ",
  "",
])
  add("mode", { text });
for (const payload of [
  '{"exp":1800003600}',
  '{"exp":1800003600.0}',
  '{"exp":1e3}',
  '{"exp":"123"}',
  '{"exp":null}',
  '{"exp":-1}',
  '{"exp":9223372036854775807}',
  '{"exp":9223372036854775808}',
  '{"exp":-9223372036854775808}',
  '{"exp":1,"exp":2}',
  "[]",
  "null",
  '{"exp":1}x',
]) {
  const encoded = Buffer.from(payload).toString("base64url");
  for (const token of [
    `a.${encoded}.`,
    `a.${encoded}`,
    `a.${encoded}=.=`,
    `a.${encoded.slice(0, 2)}=${encoded.slice(2)}.`,
    `a.${encoded}!.`,
  ])
    add("jwt", { text: token });
}
for (const value of [
  null,
  [],
  {},
  { name: "", value: "x" },
  { name: " token ", value: " secret " },
  { name: "a", value: 1 },
  { name: "a", value: "", path: "  ", secure: false, discard: true },
  ...[null, -1, 0, 1800000000, 1800000001, 1.5, "12"].map((expires) => ({
    name: "a",
    value: "b",
    expires,
  })),
  {
    name: "a",
    value: "b",
    domain: " domain ",
    path: " /path ",
    secure: "false",
  },
])
  add("cookie", { value });
const identity = {
  title: "Fixture CNKI Article",
  authors: "Ada Lovelace; Grace Hopper",
  journal_title: "Fixture CNKI Journal",
};
for (const actual of [
  identity,
  { ...identity, title: "fixture-cnki article" },
  { ...identity, authors: "Grace Hopper; Ada Lovelace" },
  { ...identity, authors: "Ada Lovelace，Grace Hopper" },
  { ...identity, title: "" },
  { ...identity, journal_title: "" },
  { ...identity, authors: "" },
])
  add("identity", { expected: identity, actual });
for (const mode of [
  "success",
  "start_failure",
  "timeout",
  "poll_failure",
  "warmup_failure",
  "fulltext_mismatch",
  "fulltext_failure",
])
  add("sequence", {
    mode,
    operations: [
      { op: "save" },
      { op: "poll" },
      { op: "warm" },
      { op: "start" },
      { op: "poll" },
      { op: "warm" },
      { op: "fresh" },
      { op: "download", identity },
      { op: "download", identity, limit: 0 },
      { op: "warm" },
      { op: "fresh", at: 1800003600 },
      { op: "load", state: {} },
      { op: "save" },
    ],
  });
for (const offset of [-1, 0, 299, 300, 301, 3599, 3600, 3601]) {
  const token = `a.${Buffer.from(JSON.stringify({ exp: 1800000000 + offset })).toString("base64url")}.`;
  add("sequence", {
    state: {
      bff_user_token: token,
      fulltext_warmed_at: 1800000000,
      cookies: [{ name: "vpn358_sid", value: "x" }],
    },
    operations: [{ op: "fresh" }, { op: "warm" }],
  });
  add("sequence", {
    state: {
      fulltext_warmed_at: 1800000000 - offset,
      final_zyproxy_url: " custom ",
      cookies: [{ name: "vpn358_sid", value: "x" }],
    },
    operations: [{ op: "fresh" }, { op: "warm" }],
  });
}
for (const state of [
  null,
  [],
  {},
  {
    bff_user_token: " bad-jwt ",
    qr_uuid: " qr ",
    fulltext_warmed_at: 1800000000,
    final_zyproxy_url: " url ",
    cookies: [
      { name: "vpn358_sid", value: "old", expires: 1 },
      { name: "vpn358_sid", value: "new" },
    ],
  },
  {
    bff_user_token: 5,
    qr_uuid: false,
    fulltext_warmed_at: 1.5,
    cookies: [null, {}, { name: "a", value: "b" }],
  },
  {
    bff_user_token: "token",
    fulltext_warmed_at: 1800000000,
    cookies: [{ name: "vpn358_sid", value: "x", expires: 1800000000 }],
  },
])
  add("sequence", {
    state,
    operations: [
      { op: "save" },
      { op: "fresh" },
      { op: "warm" },
      { op: "start" },
      { op: "save" },
    ],
  });
const proxy = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn";
const base = proxy + "/kns55/default.aspx";
for (const text of [
  "",
  "<title>T - CNKI - 中国知网</title>",
  "<h1>First</h1><h1>Second</h1><h2>Other</h2>",
  '<meta name="citation_title" content="Meta &amp;lt;A&amp;gt;"><h1>Title</h1>',
  '<META NAME="CITATION_AUTHOR" CONTENT="A"><meta name="citation_author" content="B"><meta name="citation_journal_title" content="J">',
  '<h3 id="authorpart"><span>A<sup>1</sup></span><span>B</span></h3>',
  "<h3 id='authorpart'><span>A</span></h3>",
  '<h3 id="authorpart"><span>A<span>B</span>C</span><span>D</span></h3>',
  '<p>【作者】<a href="a">A</a><a href="b">B</a></p><p>【文献出处】<a href="j">Wrong</a><span id="jname">Journal</span></p>',
  "作者：First\n作者:Second\n来源：Journal",
  '<p>【来源】<a href="">Missing href</a> Plain</p>',
  "<H1>Title</H1><p>【刊名】Journal</P>tail",
  '<meta name ="citation_title" content="ignored"><h10>Ten</h1>',
  "<h1> </h1><h1>ignored</h1><h2>Two</h2>",
]) {
  for (const variant of [
    text,
    text.toUpperCase(),
    text.replaceAll('="', '= "'),
    text.replaceAll('"', "'"),
  ])
    add("html", { text: variant });
}
for (const href of [
  "/kns55/detail/detail.aspx?FileName=A&DbName=D&DbCode=C",
  "/KNS55/DETAIL/DETAIL.ASPX?FileName=&FileName=B&DbName=one%2Btwo",
  "/kns55/detail/detail.aspx?FileName=&filename=x",
  proxy + "/kns55/detail/detail.aspx",
  "https://evil.test/kns55/detail/detail.aspx",
  "/kns55/detail/detail.aspx#",
  "/kns55/%2f/detail/detail.aspx",
  "//http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/detail/detail.aspx",
]) {
  for (const body of [
    "Title",
    "",
    "<script>ReplaceJiankuohao('JS title')</script>",
    "&amp;lt;Hi&amp;gt;",
  ])
    add("search", {
      base,
      text: `<tr><a href="${href}">${body}</a><a href="/kcms/download.aspx?filetitle=X&amp;filename=A">CAJ</a></tr><a href="${href}">Duplicate</a>`,
    });
}
for (const text of [
  '<a href="/kcms/download.aspx">PDF</a>',
  '<a href="/kcms/download.aspx">CAJ</a>',
  '<a href="/kcms/download.aspx?dflag=pdfdown">CAJ</a>',
  '<a href="https://evil.test/download.aspx">PDF</a>',
  '<a href="&#xA; /kcms/download.aspx?dflag=pdfdown&#xA; "><b>pdf下载</b></a>',
  '<a href="/kcms/download.aspx#">PDF</a>',
])
  add("pdf", { base, text });
for (const [family, origin] of [
  ["Www", "https://www.zjlib.cn"],
  ["Share", "https://share.zjlib.cn"],
  ["ZyproxyLogin", "https://login.elib.zyproxy.zjlib.cn"],
  ["Zyproxy", proxy],
]) {
  for (const suffix of [
    "",
    "/",
    "/x?y=1",
    "/x#",
    "/x#frag",
    "/%2f",
    "/%2efoo",
    "/%2e%2e/x",
    "/a/../b",
    "/%5C",
    "/%252f",
    "/汉字",
  ])
    add("url", { family, text: origin + suffix });
  for (const text of [
    origin.replace("https:", "http:"),
    origin.replace("://", "://user:@"),
    origin + ":443/x",
    origin + ":444/x",
    "invalid",
    "//evil.test",
    "https://share.zjlib.cn/",
  ])
    add("url", { family, text });
  for (const text of [
    "relative",
    "../next",
    "?x=1",
    "#",
    "//evil.test/x",
    "/%2f",
    "https://www.zjlib.cn/",
    "https://[",
  ])
    add("url", { family, base: origin + "/path/start", text });
}
for (const text of [
  "",
  String.raw`\u0041\u+041\uD83D\uDE00\u中abc\u12`,
  String.raw`\/\"\'\\\b\f\n\r\t\x41\q`,
  "trailing\\",
  'var sign="abc";var url="https:\\/\\/share.zjlib.cn\\/entry";',
  "var signature = 'other'; var sign='actual';",
  String.raw`var sign='a\'b'; var url="x";`,
  "var sign=unknown; var sign='second';",
])
  add("javascript", { text });
for (const reference of [
  "/entry",
  "https://[",
  "https://",
  "https://host:99999/",
  "https://host:no/",
  "https://bad host/",
  "https://256.1.1.1/",
  "https://xn--/",
  "//evil.test",
  "#",
  "data:hello",
])
  add("location", {
    base: "https://share.zjlib.cn/start",
    text: `window.location.href='${reference}';`,
  });
for (const base of [
  "invalid",
  "data:text/plain,test",
  "https://[",
  "https://host:99999/",
  "https://256.1.1.1/",
])
  add("location", { base, text: "window.location='/x';" });
for (const text of [
  "",
  "location.href=4;window.location='/fallback';",
  "window.location='/first';location.href='/preferred';",
  "window.location.href='/first';window.location.href='/second';",
])
  add("location", { base: "https://share.zjlib.cn/start", text });
for (const domain of [
  undefined,
  "https://share.zjlib.cn",
  "//share.zjlib.cn",
  "https://share.zjlib.cn/",
  "https://share.zjlib.cn////",
  "https://share.zjlib.cn/path",
  "https://share.zjlib.cn?",
  "https://share.zjlib.cn#",
  "https://evil.test",
  "invalid",
  "https://user@share.zjlib.cn",
]) {
  for (const portal of [
    undefined,
    "/entry",
    "/",
    "//entry",
    "/a/../entry",
    "/a%2Fb",
    "/中文",
  ]) {
    const text = `var sign='secret'; var url='/entry/callback'; ${domain === undefined ? "" : `var domainUrl='${domain}';`} ${portal === undefined ? "" : `var portalContextPath='${portal}';`} sso-login/cookie/sync`;
    add("sync", { text });
  }
}
for (const text of [
  "",
  "var sign='x';",
  "var sign='';var url='';sso-login/cookie/sync",
  "var sign='x';var url='https://evil.test/';sso-login/cookie/sync",
  "var sign='x';var url='/';",
  "var sign='x';var url='/#';sso-login/cookie/sync",
])
  add("sync", { text });
for (const text of ["", "中文 & spaces + %", "Article; punctuation", "\n\t"])
  add("forms", { text });
for (const location of [
  null,
  "",
  "/proxy/next#fragment",
  "https://[",
  "/proxy/a\\b",
  "/proxy/a b",
  "/proxy/x?q=a b",
  "/proxy/%GG",
  "file:///tmp/a",
  "ftp://example.com/a",
  "/share/escape",
  "/proxy/中文",
  "{base}/proxy/final",
  "/proxy/path?bad=%GG",
]) {
  add("redirect_wire", { location });
  add("redirect_wire", { location, redirect: false });
}
const result = spawnSync(
  "output/migration/execution/sources-zjlib-oracle.exe",
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
  "tests/migration/sources/zjlib-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-zjlib-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-zjlib.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original Rust ZJLib observations`);
