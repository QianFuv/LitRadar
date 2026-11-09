/** Smoke the isolated client image without any server data or secrets. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { randomUUID } from "node:crypto";

const IMAGE = process.argv[2];
assert(IMAGE, "Usage: node apps/client/tests/client-web-smoke.mjs <image>");
const NAME = `litradar-client-smoke-${randomUUID()}`;

/** Run a bounded Docker command against task-owned resources. */
function docker(...args) {
  return execFileSync("docker", args, {
    encoding: "utf8",
    timeout: 60000,
    windowsHide: true,
  });
}

try {
  docker(
    "run",
    "-d",
    "--name",
    NAME,
    "--read-only",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges",
    "-p",
    "127.0.0.1::8080",
    IMAGE,
  );
  const port = docker("port", NAME, "8080/tcp").trim().split(":").at(-1);
  const origin = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 60000;
  let index;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(origin, {
        signal: AbortSignal.timeout(3000),
      });
      if (response.ok) {
        index = await response.text();
        break;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  assert(index?.includes('id="root"'), "Client index unavailable");
  const login = await fetch(`${origin}/login`, {
    signal: AbortSignal.timeout(5000),
  });
  assert.equal(login.status, 200);
  assert.equal(
    await login.text(),
    index,
    "Client routes must serve the SPA index",
  );
  const inspection = JSON.parse(docker("image", "inspect", IMAGE))[0];
  fs.mkdirSync("release-results/client", { recursive: true });
  fs.writeFileSync(
    "release-results/client/smoke.json",
    JSON.stringify(
      {
        status: "passed",
        imageId: inspection.Id,
        endpoints: ["/", "/login"],
      },
      null,
      2,
    ) + "\n",
  );
  console.log("Client image smoke passed: / and /login serve index.html");
} finally {
  docker("rm", "-f", NAME);
}
