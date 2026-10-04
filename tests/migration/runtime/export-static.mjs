/** Capture production frontend file semantics from the immutable Rust application. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { gzipSync } from "node:zlib";
import { BASELINE, digest, loadOracle } from "../oracle.mjs";

const oracle = await loadOracle(BASELINE);
const root = await fs.mkdtemp(
  path.join(os.tmpdir(), "litradar-runtime-static-"),
);
const modified = "Mon, 03 Aug 2026 12:00:00 GMT";
const files = {
  "index.html": "<html>home</html>",
  "404.html": "<html>missing</html>",
  "page.html": "<html>page</html>",
  "folder/index.html": "<html>folder</html>",
  "asset.js": "console.log('fixture');\n",
  "asset.css": "body{color:red}\n",
  "route.txt": "route data\n",
  "font.woff2": "font data",
  "unknown.xyzabc": "unknown data",
  ".hidden": "hidden data",
  "empty.txt": "",
  "source.map": "source map",
  "module.mjs": "module script",
  "font.woff": "legacy font",
  "font.otf": "open font",
  "site.webmanifest": "{}",
  "assets.css/index.html": "<html>directory</html>",
};
let child, exited;
try {
  await fs.writeFile(
    path.join(root, ".litradar-e2e-root"),
    "litradar-full-stack-e2e-v1\n",
  );
  await fs.writeFile(path.join(root, "secret.key"), Buffer.alloc(32, 42), {
    mode: 0o600,
  });
  const fixture = spawnSync(
    path.join(oracle.directory, oracle.manifest.binaries.fullStackFixture),
    ["--project-root", root],
    { cwd: root, timeout: 30000, encoding: "utf8", windowsHide: true },
  );
  assert.ifError(fixture.error);
  assert.equal(fixture.status, 0, fixture.stderr);
  const manifestFiles = [];
  for (const [name, content] of Object.entries(files)) {
    const filename = path.join(root, "web", name);
    await fs.mkdir(path.dirname(filename), { recursive: true });
    await fs.writeFile(filename, content);
    await fs.utimes(filename, new Date(modified), new Date(modified));
    if (name.endsWith(".html"))
      manifestFiles.push({
        path: name,
        html_sha256: `sha256-${Buffer.from(digest(Buffer.from(content)), "hex").toString("base64")}`,
        inline_script_hashes: [],
      });
  }
  const compressed = gzipSync(files["asset.js"]);
  await fs.writeFile(path.join(root, "web/asset.js.gz"), compressed);
  await fs.utimes(
    path.join(root, "web/asset.js.gz"),
    new Date(modified),
    new Date(modified),
  );
  await fs.writeFile(
    path.join(root, "web/csp-hashes.json"),
    JSON.stringify({
      version: 1,
      algorithm: "sha256",
      files: manifestFiles.sort((first, second) =>
        first.path.localeCompare(second.path),
      ),
      script_hashes: [],
    }),
  );
  const reserved = net.createServer();
  await new Promise((resolve, reject) => {
    reserved.once("error", reject);
    reserved.listen(0, "127.0.0.1", resolve);
  });
  const port = reserved.address().port;
  await new Promise((resolve) => reserved.close(resolve));
  child = spawn(
    path.join(oracle.directory, oracle.manifest.binaries.application),
    [
      "serve",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--project-root",
      root,
      "--secret-key-file",
      path.join(root, "secret.key"),
      "--scheduler-interval-seconds",
      "3600",
    ],
    { cwd: root, stdio: "ignore", windowsHide: true },
  );
  exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", resolve);
  });
  let isReady = false;
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null, "original server exited during startup");
    try {
      isReady = (
        await fetch(`http://127.0.0.1:${port}/health/ready`, {
          signal: AbortSignal.timeout(500),
        })
      ).ok;
    } catch {}
    if (isReady) break;
    await delay(100);
  }
  assert(isReady, "original production startup timeout");
  const scenarios = [];
  for (const route of [
    "/",
    "/index.html",
    "/page?query=1",
    "/page/",
    "/folder",
    "/folder/",
    "/folder/index.html",
    "/asset.js",
    "/asset.css",
    "/route.txt",
    "/font.woff2",
    "/unknown.xyzabc",
    "/.hidden",
    "/missing",
    "/api/missing",
    "/%2e%2e/asset.js",
    "/%2e%2e%2fasset.js",
    "/%2e%2e%5casset.js",
    "/%61sset.js",
    "/bad%FF",
    "/asset.js%00",
    "//asset.js",
    "/source.map",
    "/module.mjs",
    "/font.woff",
    "/font.otf",
    "/site.webmanifest",
    "/assets.css?x=1",
    "/%2e%2e/%00file.js",
  ]) {
    for (const method of ["GET", "HEAD", "POST", "OPTIONS"])
      scenarios.push({ path: route, method, headers: {} });
  }
  for (const headers of [
    { "accept-encoding": "gzip" },
    { "accept-encoding": "gzip;q=0" },
    { "accept-encoding": "*" },
    { "accept-encoding": "GZIP" },
    { "accept-encoding": "gzip;q=0.1, identity;q=1" },
    { "accept-encoding": "gzip;q=1, identity;q=0" },
    { "accept-encoding": "gzip;q=bad" },
    { range: "bytes=0-3" },
    { range: "bytes=-5" },
    { range: "bytes=4-" },
    { range: "bytes=999-" },
    { range: "bytes=0-1,4-5" },
    { range: "invalid" },
    { range: "bytes=5-2" },
    { range: "bytes=0-999" },
    { "if-modified-since": modified },
    { "if-modified-since": "Mon, 01 Jan 2024 00:00:00 GMT" },
    { "if-unmodified-since": "Mon, 01 Jan 2024 00:00:00 GMT" },
    { range: "bytes=0-3", "if-range": modified },
    { range: "bytes=0-3", "if-range": "Mon, 01 Jan 2024 00:00:00 GMT" },
    { range: "bytes=0-3", "if-range": '"tag"' },
    { range: "bytes=0-3", "accept-encoding": "gzip" },
    { cookie: "litradar_session=fixture" },
    { "accept-encoding": "x-gzip;Q=0.999" },
    { "accept-encoding": "gzip;q=0.9999" },
    { "accept-encoding": "\u00a0gzip" },
    { range: "bytes=0-1,\u00a03-4" },
    { range: "bytes=00-1" },
    { range: "bytes=0-1, 3-4" },
    { range: "bytes=0-1,3-4," },
    { range: "bytes=-99" },
    { range: "bytes=-0" },
    { range: "bytes=0-3,2-4" },
    { range: "bytes=0-1,99-" },
    { "if-match": '"6a7082c0.00000000-18"' },
    { "if-match": 'W/"6a7082c0.00000000-18"' },
    { "if-match": "*", "if-unmodified-since": "Mon, 01 Jan 2024 00:00:00 GMT" },
    { "if-none-match": 'W/"6a7082c0.00000000-18"' },
    { "if-none-match": '"other,tag", "6a7082c0.00000000-18"' },
    { "if-none-match": '"other"', "if-modified-since": modified },
    { "if-unmodified-since": "Sun, 07 Nov 1994 08:48:37 GMT" },
    { "if-unmodified-since": "Mon, 07 Nov 1994 08:48:37 GMT" },
    { "if-modified-since": "Sunday, 06-Nov-69 08:49:37 GMT" },
    { "if-modified-since": "Wednesday, 06-Nov-69 08:49:37 GMT" },
    { "if-modified-since": "Mon Aug  3 12:00:00 2026" },
    { "if-modified-since": "Mon Aug 03 12:00:00 2026" },
  ])
    for (const method of ["GET", "HEAD"])
      scenarios.push({ path: "/asset.js", method, headers });
  for (const range of ["bytes=0-0", "bytes=0-", "bytes=-1"])
    for (const method of ["GET", "HEAD"])
      scenarios.push({ path: "/empty.txt", method, headers: { range } });
  scenarios.push(
    { path: "/missing", method: "GET", headers: { range: "bytes=0-3" } },
    {
      path: "/missing",
      method: "GET",
      headers: { "if-modified-since": modified },
    },
    {
      path: "/page",
      method: "GET",
      headers: { cookie: "litradar_session=fixture" },
    },
  );
  const cases = [];
  for (const scenario of scenarios) {
    const response = await new Promise((resolve, reject) => {
      const request = http.request(
        { host: "127.0.0.1", port, ...scenario },
        (response) => {
          const chunks = [];
          response.on("data", (chunk) => chunks.push(chunk));
          response.on("end", () =>
            resolve({
              status: response.statusCode,
              headers: response.headers,
              body: Buffer.concat(chunks),
            }),
          );
          response.on("error", reject);
        },
      );
      request.setTimeout(5000, () =>
        request.destroy(new Error("static request timeout")),
      );
      request.on("error", reject);
      request.end();
    });
    const headers = {};
    for (const name of [
      "content-type",
      "content-length",
      "content-encoding",
      "content-range",
      "accept-ranges",
      "last-modified",
      "etag",
      "vary",
      "allow",
      "location",
      "cache-control",
    ])
      if (response.headers[name] !== undefined)
        headers[name] = response.headers[name];
    cases.push({
      ...scenario,
      response: {
        status: response.status,
        headers,
        body_sha256: digest(response.body),
        body_length: response.body.length,
      },
    });
  }
  await fs.writeFile(
    "tests/migration/runtime/static-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/runtime/export-static.mjs"),
        ),
        modified,
        files,
        gzip_base64: compressed.toString("base64"),
        cases,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify({ static_original_responses: cases.length }));
} finally {
  if (child && child.exitCode === null) child.kill();
  if (exited)
    await Promise.race([
      exited,
      delay(10000, undefined, { ref: false }).then(() => {
        throw new Error("original production shutdown timeout");
      }),
    ]);
  const resolved = path.resolve(root);
  assert.equal(path.dirname(resolved), path.resolve(os.tmpdir()));
  assert(path.basename(resolved).startsWith("litradar-runtime-static-"));
  await fs.rm(resolved, { recursive: true, force: true });
}
