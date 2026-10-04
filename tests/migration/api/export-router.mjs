/** Capture the original router and middleware contract through raw HTTP paths. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { BASELINE, digest, loadOracle } from "../oracle.mjs";

const oracle = await loadOracle(BASELINE);
const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-api-router-"));
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
      "--development",
    ],
    { cwd: root, stdio: "ignore", shell: false, windowsHide: true },
  );
  exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", resolve);
  });
  let ready = false;
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null);
    try {
      ready = (
        await fetch(`http://127.0.0.1:${port}/health/ready`, {
          signal: AbortSignal.timeout(500),
        })
      ).ok;
    } catch {}
    if (ready) break;
    await delay(100);
  }
  assert(ready, "original API startup timeout");
  const scenarios = [];
  for (const route of [
    "/health/live",
    "/api/auth/me",
    "/api/favorites/folders/1/articles/page",
    "/docs?old=1",
    "/docs/",
    "/docs//",
    "/docs/missing",
    "/api/%61rticles",
    "/api/articles/%31",
    "/api/articles/1/",
    "/api//articles",
    "/api/../health/live",
    "/secret-path/?token=private",
    "/missing",
    "/api/missing",
    "/mcp",
    "/mcp/",
    "/mcp/nested",
  ]) {
    for (const method of ["GET", "HEAD", "DELETE", "OPTIONS"])
      scenarios.push({ method, path: route, headers: {} });
  }
  for (const headers of [
    { authorization: "" },
    { cookie: "litradar_session=" },
    { cookie: "other=1" },
    {
      origin: "https://unlisted.example",
      "access-control-request-method": "PATCH",
      "access-control-request-headers": "authorization,content-type",
    },
  ]) {
    for (const method of ["GET", "OPTIONS"])
      scenarios.push({ method, path: "/health/live", headers });
  }
  const assets = (await fs.readdir("internal/openapi/swagger")).filter(
    (name) =>
      ![".gitattributes", "LICENSE", "NOTICE", "README.md"].includes(name),
  );
  assert.equal(assets.length, 18);
  for (const name of [
    ...assets,
    "sw%61gger-initializer.js",
    "LICENSE",
    "NOTICE",
    "../index.html",
  ])
    scenarios.push({ method: "GET", path: `/docs/${name}`, headers: {} });
  const cases = [];
  const inventory = JSON.parse(
    await fs.readFile("tests/data/migration/inventory.json", "utf8"),
  );
  const routes = [
    ...new Set(inventory.operations.map((operation) => operation.path)),
  ];
  assert.equal(routes.length, 71);
  for (const route of routes)
    scenarios.push({
      method: "TRACE",
      path: route.replace(/\{[^}]+\}/g, "1"),
      headers: {},
    });
  for (const method of ["GET", "HEAD", "DELETE", "OPTIONS"])
    scenarios.push({ method, path: "/docs/%FF", headers: {} });
  for (const scenario of scenarios) {
    const observed = await new Promise((resolve, reject) => {
      const request = http.request(
        {
          host: "127.0.0.1",
          port,
          method: scenario.method,
          path: scenario.path,
          headers: {
            ...scenario.headers,
            "x-request-id": "untrusted-client-id",
          },
        },
        (response) => {
          const chunks = [];
          response.on("data", (chunk) => chunks.push(chunk));
          response.once("error", reject);
          response.once("end", () =>
            resolve({
              status: response.statusCode,
              headers: response.headers,
              body: Buffer.concat(chunks),
            }),
          );
        },
      );
      request.setTimeout(10000, () =>
        request.destroy(new Error("original router request timeout")),
      );
      request.once("error", reject);
      request.end();
    });
    assert.match(
      observed.headers["x-request-id"],
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
    const headers = Object.fromEntries(
      Object.entries(observed.headers).filter(
        ([name]) =>
          ![
            "date",
            "x-request-id",
            "connection",
            "keep-alive",
            "transfer-encoding",
            "content-length",
          ].includes(name),
      ),
    );
    if (
      scenario.method === "HEAD" &&
      observed.headers["content-length"] !== undefined
    )
      headers["content-length"] = observed.headers["content-length"];
    cases.push({
      ...scenario,
      response: {
        status: observed.status,
        headers,
        body_sha256: digest(observed.body),
        body_length: observed.body.length,
      },
    });
  }
  await fs.writeFile(
    "tests/migration/api/router-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-router.mjs"),
        ),
        cases,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify({ router_original_responses: cases.length }));
} finally {
  if (child && child.exitCode === null) child.kill();
  if (exited)
    await Promise.race([
      exited,
      delay(10000, undefined, { ref: false }).then(() => {
        throw new Error("original API shutdown timeout");
      }),
    ]);
  const resolved = path.resolve(root);
  assert.equal(path.dirname(resolved), path.resolve(os.tmpdir()));
  assert(path.basename(resolved).startsWith("litradar-api-router-"));
  await fs.rm(resolved, { recursive: true, force: true });
}
