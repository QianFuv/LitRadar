/** Freeze full public CNKI indexing with original fixture HTML and durable progress replay. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const source = await fs.readFile(
  "crates/litradar-sources/src/cnki_domestic.rs",
  "utf8",
);
const html = {};
for (const name of ["SEARCH", "DETAIL", "YEAR", "PAPERS", "ABSTRACT"]) {
  const match = source.match(
    new RegExp(`const ${name}_HTML: &str = r#"([\\s\\S]*?)"#;`),
  );
  assert.ok(match);
  html[name] = match[1];
}
const catalog = {
  catalog_id: "cnki",
  catalog_aliases: [],
  title: "世界经济",
  issn: "1002-9621",
  eissn: null,
  all_issns: ["1002-9621"],
  title_aliases: [],
  area: null,
  rankings: {},
};
const fixture = {
  journal_search_html: html.SEARCH,
  journal_detail_html: html.DETAIL,
  year_issues_html: html.YEAR,
  issue_article_pages: {
    202512: [html.PAPERS],
    202511: ['<input id="articleCount" value="0">'],
  },
  article_detail_html: { id: html.ABSTRACT, SJJJ202512002: html.ABSTRACT },
  article_detail_status_codes: {},
  fail_endpoint: null,
};
const cases = [];
/** Register an independent end-to-end source run. */
function add(input) {
  cases.push({
    id: `cnki-flow-${cases.length}`,
    kind: "cnki_workflow",
    input: JSON.stringify({ catalog, fixture, mode: "bootstrap", ...input }),
  });
}
for (const mode of ["bootstrap", "incremental", "full_rescan"])
  for (const issue of [null, "202512", "202511", "202510"])
    add({
      mode,
      anchor: issue
        ? JSON.stringify({ version: 1, year_issue_id: issue })
        : null,
    });
for (const workers of [2, 4, 32]) add({ workers });
add({ replay_at: [0] });
add({
  fixture: { ...fixture, year_issues_html: '<div id="YearIssueTree"></div>' },
});
for (const status of [404, 410, 500])
  add({
    fixture: {
      ...fixture,
      article_detail_status_codes: { id: status, SJJJ202512002: status },
    },
  });
add({
  fixture: {
    ...fixture,
    article_detail_html: {
      id: '<input id="paramfilename" value="id"><h1 class="title">Title</h1>',
      SJJJ202512002:
        '<input id="paramfilename" value="id"><h1 class="title">Title</h1>',
    },
  },
});
const checkpoint = {
  version: 2,
  base_anchor_issue_id: null,
  candidate_head_issue_id: "202512",
  current_issue_id: "202511",
  page_index: 0,
};
add({ checkpoint: JSON.stringify(checkpoint) });
for (const field of [
  "version",
  "candidate_head_issue_id",
  "current_issue_id",
  "base_anchor_issue_id",
])
  add({
    checkpoint: JSON.stringify({
      ...checkpoint,
      [field]: field === "version" ? 1 : "missing",
    }),
  });
add({ anchor: '{"version":1,"year_issue_id":"token"}' });
add({ catalog: { ...catalog, title: "missing", issn: null, all_issns: [] } });
const result = spawnSync(
  "output/migration/execution/sources-index-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout.trim().split(/\r?\n/);
assert.equal(observations.length, cases.length);
const metadata = {
  provenance: JSON.parse(
    await fs.readFile(
      "output/migration/execution/t05-index-oracle-build.json",
      "utf8",
    ),
  ),
  exporter_sha256: digest(
    await fs.readFile("tests/migration/sources/export-cnki-flows.mjs"),
  ),
};
await fs.writeFile(
  "tests/migration/sources/cnki-flow-vectors.json",
  JSON.stringify(metadata, null, 2).replace(
    /\n}$/,
    `,\n"observations":[\n${observations.join(",\n")}\n]\n}\n`,
  ),
);
console.log(`Frozen ${observations.length} original Rust CNKI workflows`);
