/** Verify script attribute admission and exact-byte CSP manifests independently. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  buildCspManifest,
  extractInlineScriptHashes,
  generateCspManifest,
} from "../scripts/generate-csp.mjs";

/** Calculate the expected standard CSP hash directly from fixture bytes. */
function expectedHash(value) {
  return `sha256-${createHash("sha256").update(value).digest("base64")}`;
}

/** Exclude genuine source attributes regardless of spelling or assigned value. */
function excludesExternalScripts() {
  for (const attributes of [
    'SRC="/external.js"',
    "src",
    'src=""',
    "/src",
    "src/foo",
    "=src",
    "x='value'src",
    "x=plain src=/external.js",
    "\u00a0src",
    "\ufeffsrc",
  ]) {
    assert.deepEqual(
      extractInlineScriptHashes(`<script ${attributes}>external</script>`),
      [],
      attributes,
    );
  }
}

/** Include lookalike names and source text inside quoted or unquoted values. */
function includesInlineScripts() {
  const body = "\r\nglobalThis.text = '中文 > <tag>';\n";
  for (const attributes of [
    "data-src='/external.js'",
    "ſrc='/external.js'",
    'title="src=x"',
    "title='> src=/external.js'",
    "x=abc/src=x",
    "x==src",
  ]) {
    assert.deepEqual(
      extractInlineScriptHashes(`<script ${attributes}>${body}</script>`),
      [expectedHash(body)],
      attributes,
    );
  }
}

/** Preserve document order, duplicates and the existing comment-contained grammar. */
function preservesScriptSequence() {
  const first = "globalThis.first = '<tag>';";
  const second = "globalThis.second = '中文';";
  const html = `<SCRIPT data-src='ignored'>${first}</SCRIPT><script src='/external.js'>ignored</script><script>${second}</script><!-- <script>comment script</script> --><script title='src=x'>same</script><script ſrc='x'>same</script><script src>ignored</script>`;
  assert.deepEqual(extractInlineScriptHashes(html), [
    expectedHash(first),
    expectedHash(second),
    expectedHash("comment script"),
    expectedHash("same"),
    expectedHash("same"),
  ]);
  assert.deepEqual(
    extractInlineScriptHashes("<scripture>ignore</scripture><script"),
    [],
  );
}

/** Reject malformed opening and closing tags before admitting external scripts. */
function rejectsMalformedScripts() {
  assert.throws(
    () => extractInlineScriptHashes("<script title='unterminated>"),
    { message: "Static HTML contains an unterminated script opening tag" },
  );
  for (const html of [
    "<script>not closed",
    "<script src='/external.js'>not closed",
    "<script>text</script / >",
  ]) {
    assert.throws(() => extractInlineScriptHashes(html), {
      message: "Static HTML contains an unterminated script element",
    });
  }
}

/** Verify sorted file inventory, exact bytes and unique aggregate manifest hashes. */
async function preservesManifestBytes(context) {
  const directory = await fs.mkdtemp(
    path.join(os.tmpdir(), "litradar-csp-test-"),
  );
  /** Remove only the absolute temporary directory created by this fixture. */
  async function removeFixture() {
    assert.equal(path.dirname(directory), path.resolve(os.tmpdir()));
    assert(path.basename(directory).startsWith("litradar-csp-test-"));
    await fs.rm(directory, { recursive: true, force: true });
  }
  context.after(removeFixture);
  const first = "<script>b</script>";
  const second = "<script>a</script><script>a</script>";
  await fs.writeFile(path.join(directory, "z.HTML"), second);
  await fs.writeFile(path.join(directory, "a.html"), first);
  const expected = {
    version: 1,
    algorithm: "sha256",
    files: [
      {
        path: "a.html",
        html_sha256: expectedHash(first),
        inline_script_hashes: [expectedHash("b")],
      },
      {
        path: "z.HTML",
        html_sha256: expectedHash(second),
        inline_script_hashes: [expectedHash("a"), expectedHash("a")],
      },
    ],
    script_hashes: [expectedHash("a"), expectedHash("b")].sort(),
  };
  assert.deepEqual(await buildCspManifest(directory), expected);
  const filename = await generateCspManifest(directory);
  assert.equal(filename, path.join(directory, "csp-hashes.json"));
  assert.equal(
    await fs.readFile(filename, "utf8"),
    `${JSON.stringify(expected, null, 2)}\n`,
  );
}

test("excludes real src attributes", excludesExternalScripts);
test(
  "includes quoted source text and lookalike attributes",
  includesInlineScripts,
);
test(
  "preserves exact script bytes and duplicate sequence",
  preservesScriptSequence,
);
test(
  "rejects malformed tags including external scripts",
  rejectsMalformedScripts,
);
test(
  "generates the independently specified CSP manifest",
  preservesManifestBytes,
);
