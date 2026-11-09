/** Verify the shipped gateway topology using only task-owned volumes and a synthetic key. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { randomBytes } from "node:crypto";

const ROOT = fs.mkdtempSync(path.join(os.tmpdir(), "litradar-compose-"));
const PROJECT = `litradar-smoke-${randomBytes(6).toString("hex")}`;
const CONFIG = path.join(ROOT, "compose.json");
const PASSWORD = randomBytes(24).toString("base64url") + "Aa1!";
/**
 * Compose bind-mounts file secrets with host ownership, so the synthetic key must be readable
 * by the image's non-root account (10001) without a host chown.
 */
const CONTAINER_READABLE_KEY_MODE = 0o644;

/** Run a bounded Docker command, keeping the synthetic password on stdin. */
function docker(args, input) {
  return execFileSync("docker", args, {
    input,
    encoding: "utf8",
    timeout: 180000,
    windowsHide: true,
  });
}

/** Invoke Compose against the isolated project, never the repository's data or secrets. */
function compose(...args) {
  return docker(["compose", "-p", PROJECT, "-f", CONFIG, ...args]);
}

try {
  const configuration = JSON.parse(
    docker([
      "compose",
      "-f",
      "docker-compose.yml",
      "config",
      "--format",
      "json",
    ]),
  );
  assert.deepEqual(Object.keys(configuration.services).sort(), [
    "client-web",
    "gateway",
    "server",
  ]);
  assert(
    !configuration.services.server.ports &&
      !configuration.services["client-web"].ports,
  );
  assert(
    !configuration.services["client-web"].volumes &&
      !configuration.services["client-web"].secrets,
  );
  configuration.name = PROJECT;
  configuration.services.server.image =
    process.argv[2] || "litradar-wave2-server";
  configuration.services["client-web"].image =
    process.argv[3] || "litradar-wave2-client";
  delete configuration.services.server.build;
  delete configuration.services["client-web"].build;
  configuration.services.server.volumes = [
    { type: "volume", source: "smoke-data", target: "/app/data" },
  ];
  configuration.volumes = { "smoke-data": {} };
  delete configuration.networks.backend.ipam;
  for (const network of Object.values(configuration.networks))
    delete network.name;
  fs.writeFileSync(path.join(ROOT, "key"), randomBytes(32), {
    mode: CONTAINER_READABLE_KEY_MODE,
  });
  configuration.secrets.litradar_key.file = path.join(ROOT, "key");
  fs.copyFileSync("Caddyfile", path.join(ROOT, "Caddyfile"));
  configuration.services.gateway.volumes = [
    {
      type: "bind",
      source: path.join(ROOT, "Caddyfile"),
      target: "/etc/caddy/Caddyfile",
      read_only: true,
    },
  ];
  configuration.services.gateway.ports = [
    { target: 8000, published: "0", host_ip: "127.0.0.1", protocol: "tcp" },
  ];
  fs.writeFileSync(CONFIG, JSON.stringify(configuration));
  docker(
    [
      "compose",
      "-p",
      PROJECT,
      "-f",
      CONFIG,
      "run",
      "--rm",
      "--no-deps",
      "-T",
      "server",
      "admin",
      "bootstrap",
      "--username",
      "compose_smoke",
      "--password-stdin",
      "--project-root",
      "/app",
    ],
    PASSWORD + "\n",
  );
  compose("up", "-d", "--no-build", "--wait", "--wait-timeout", "120");
  const origin = `http://${compose("port", "gateway", "8000").trim()}`;
  const index = await fetch(origin, { signal: AbortSignal.timeout(10000) });
  assert.equal(index.status, 200);
  const html = await index.text();
  assert(html.includes('id="root"'));
  const loginPage = await fetch(`${origin}/login`);
  assert.equal(loginPage.status, 200);
  assert.equal(await loginPage.text(), html);
  for (const route of ["/health/ready", "/docs", "/docs/", "/openapi.json"]) {
    assert.equal(
      (await fetch(`${origin}${route}`, { signal: AbortSignal.timeout(10000) }))
        .status,
      200,
      route,
    );
  }
  for (const route of ["/wails", "/wails/runtime", "/wails/runtime.js"]) {
    assert.equal((await fetch(`${origin}${route}`)).status, 404, route);
  }
  const login = await fetch(`${origin}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json", origin },
    body: JSON.stringify({ username: "compose_smoke", password: PASSWORD }),
  });
  assert.equal(login.status, 200, "Cookie login through gateway");
  const cookie = login.headers.get("set-cookie")?.split(";", 1)[0];
  assert(cookie);
  assert.equal(
    (await fetch(`${origin}/api/auth/me`, { headers: { cookie } })).status,
    200,
  );
  const native = await fetch(`${origin}/api/auth/native/login`, {
    method: "POST",
    headers: { "content-type": "application/json", origin },
    body: JSON.stringify({
      username: "compose_smoke",
      password: PASSWORD,
      device_name: "compose-smoke",
    }),
  });
  assert.equal(native.status, 200, "Native login through gateway");
  const token = await native.json();
  assert.equal(
    (
      await fetch(`${origin}/api/auth/me`, {
        headers: { authorization: `Bearer ${token.token}` },
      })
    ).status,
    200,
  );
  const initialize = await fetch(`${origin}/mcp`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      authorization: `Bearer ${token.token}`,
      origin,
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "compose-smoke", version: "1" },
      },
    }),
    signal: AbortSignal.timeout(10000),
  });
  assert.equal(initialize.status, 200, "MCP initialize through gateway");
  assert(
    initialize.headers.get("mcp-session-id"),
    "MCP session header missing",
  );
  assert(
    (await initialize.text()).includes("protocolVersion"),
    "MCP response unavailable",
  );
  fs.mkdirSync("test-results/compose-smoke", { recursive: true });
  fs.writeFileSync(
    "test-results/compose-smoke/summary.json",
    JSON.stringify(
      {
        status: "passed",
        routes: ["/", "/login", "/health/ready", "/docs", "/wails/runtime"],
        cookieLogin: true,
        bearerLogin: true,
        mcpInitialize: true,
        project: PROJECT,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(
    "Compose smoke passed: client routes, server routes, Wails denial, cookie and Bearer authentication",
  );
} catch (error) {
  if (fs.existsSync(CONFIG))
    process.stderr.write(compose("logs", "--no-color", "--tail", "40"));
  throw error;
} finally {
  if (fs.existsSync(CONFIG)) compose("down", "--volumes", "--remove-orphans");
  assert(ROOT.startsWith(path.join(os.tmpdir(), "litradar-compose-")));
  fs.rmSync(ROOT, { recursive: true, force: true });
}
