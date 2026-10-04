/** Freeze original notification stdout classification and exact retention boundaries. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";
const build = JSON.parse(
  await fs.readFile(
    "output/migration/execution/index-oracle/identity-build.json",
    "utf8",
  ),
);
const payload = {
  protocol_version: 1,
  attempt_id: "attempt",
  workflow: "notify",
  mode: "execute",
  status: "completed",
  db_name: "example.sqlite",
};
const requests = [];
/** Store literal output bytes so invalid UTF-8 and duplicate keys remain observable. */
function add(name, value, exit = 0, dry = false) {
  requests.push({
    op: "notify_wire",
    name,
    bytes: typeof value === "string" ? [...Buffer.from(value)] : value,
    attempt: "attempt",
    database: "example.sqlite",
    dry,
    exit,
  });
}
for (const status of [
  "running",
  "idle",
  "completed",
  "skipped",
  "failed",
  "cancelled",
  "timed_out",
  "unknown",
  "unexpected",
]) {
  for (const exit of [null, 0, 1, -1]) {
    const value = { ...payload, status };
    add(`${status}-${exit}`, JSON.stringify(value), exit);
    add(
      `sequence-${status}-${exit}`,
      JSON.stringify(Object.values(value)),
      exit,
    );
  }
}
for (const key of Object.keys(payload)) {
  const copy = { ...payload };
  delete copy[key];
  add(`missing-${key}`, JSON.stringify(copy));
  add(`null-${key}`, JSON.stringify({ ...payload, [key]: null }));
  add(
    `duplicate-${key}`,
    JSON.stringify(payload).slice(0, -1) +
      `,"${key}":${JSON.stringify(payload[key])}}`,
  );
}
for (const [name, value] of Object.entries({
  unknown: { ...payload, extra: 1 },
  version: { ...payload, protocol_version: 2 },
  attempt: { ...payload, attempt_id: "other" },
  workflow: { ...payload, workflow: "index" },
  mode: { ...payload, mode: "dry_run" },
  database: { ...payload, db_name: "other.sqlite" },
}))
  add(name, JSON.stringify(value));
add("dry", JSON.stringify({ ...payload, mode: "dry_run" }), 0, true);
for (const [name, value] of Object.entries({
  empty: "",
  null: "null",
  number: "1",
  invalid: "!",
  truncated: JSON.stringify(payload).slice(0, -1),
  trailing: JSON.stringify(payload) + "{}",
  space: JSON.stringify(payload) + " \r\n",
  float: JSON.stringify(payload).replace(
    '"protocol_version":1',
    '"protocol_version":1.0',
  ),
}))
  add(name, value);
add("invalid-utf8", [255, 254]);
for (const size of [65535, 65536, 65537, 73729])
  add(`bounded-${size}`, JSON.stringify(payload).padEnd(size, " "));
const observations = [];
for (const input of requests) {
  const result = spawnSync(build.binary, [], {
    input: JSON.stringify(input) + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 4 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  observations.push(JSON.parse(result.stdout));
}
assert.equal(
  observations.find((value) => value.input.name === "bounded-65536").expected
    .status,
  "completed",
);
assert.equal(
  observations.find((value) => value.input.name === "bounded-65537").expected
    .status,
  "unknown",
);
await fs.writeFile(
  "tests/migration/index/notify-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-notify.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Frozen ${observations.length} original notification observations`);
