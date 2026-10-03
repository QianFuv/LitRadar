/** Freeze real reqwest response decoding using task-local raw HTTP responses. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { gzipSync } from "node:zlib";
import { digest } from "../oracle.mjs";

const requests = [];
/** Assemble exact response bytes for the loopback oracle. */
function add(id, encoding, body, limit = 1024, extra = "") {
  const headers = `HTTP/1.1 200 OK\r\nConnection: close\r\n${encoding === null ? "" : `Content-Encoding: ${encoding}\r\n`}Content-Length: ${body.length}\r\n${extra}\r\n`;
  requests.push({
    kind: "wire",
    id,
    response: [...Buffer.concat([Buffer.from(headers), body])],
    limit,
  });
}
for (const encoding of [
  null,
  "gzip",
  "GZIP",
  "GZip",
  "gzip, identity",
  "identity",
  "br",
  "deflate",
]) {
  add(`${encoding}-plain`, encoding, Buffer.from("[]"));
  add(`${encoding}-compressed`, encoding, gzipSync(Buffer.from("[]")));
}
add(
  "gzip-members",
  "gzip",
  Buffer.concat([
    gzipSync(Buffer.from("first")),
    gzipSync(Buffer.from("second")),
  ]),
);
add(
  "gzip-trailing",
  "gzip",
  Buffer.concat([gzipSync(Buffer.from("first")), Buffer.from("junk")]),
);
add("gzip-truncated", "gzip", gzipSync(Buffer.from("first")).subarray(0, 12));
add("gzip-decoded-limit", "gzip", gzipSync(Buffer.alloc(2048, 97)), 1024);
add("gzip-encoded-limit", "gzip", gzipSync(Buffer.from("[]")), 2);
add(
  "gzip-duplicate-first",
  "gzip",
  gzipSync(Buffer.from("[]")),
  1024,
  "Content-Encoding: identity\r\n",
);
add(
  "gzip-duplicate-second",
  "identity",
  Buffer.from("[]"),
  1024,
  "Content-Encoding: gzip\r\n",
);
const result = spawnSync(
  "output/migration/execution/sources-transport-oracle.exe",
  [],
  {
    input: requests.map((item) => JSON.stringify(item)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  },
);
assert.equal(result.status, 0, result.stderr || String(result.error));
assert.ifError(result.error);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, requests.length);
await fs.writeFile(
  "tests/migration/sources/wire-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-transport-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-wire.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust HTTP wire observations`,
);
