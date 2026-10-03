/** Freeze key selection, quota, cooldown and saturation against original schedulers. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { gzipSync } from "node:zlib";
import { digest } from "../oracle.mjs";

/** Encode exact Rust Duration units without passing u64 values through Number. */
function duration(milliseconds) {
  const value = BigInt(milliseconds);
  return {
    seconds: String(value / 1000n),
    nanoseconds: Number(value % 1000n) * 1000000,
  };
}
const maximum = { seconds: "18446744073709551615", nanoseconds: 999999999 };
const cases = [];
for (const service of ["openalex", "semantic_scholar"]) {
  for (const keys of [0, 1, 2, 3, 8]) {
    for (const processCount of [0, 1, 2, 9, "18446744073709551615"]) {
      for (const processId of [0, 1, 7, "18446744073709551615"]) {
        cases.push({
          id: `${service}-cohort-${keys}-${processCount}-${processId}`,
          service,
          keys,
          process_id: String(processId),
          process_count: String(processCount),
          capacity: "32",
          epoch: duration(123456),
          interval: duration(1100),
          operations: [0, 0, 0, 1, 39, 40, 1099, 1100, 1000000].map(
            (offset) => ({ op: "reserve", now: duration(123456 + offset) }),
          ),
        });
      }
    }
  }
}
for (const service of ["openalex", "semantic_scholar"]) {
  const healths = [
    "Success",
    "AuthenticationFailure",
    "RateLimited",
    "TransientFailure",
    "TerminalFailure",
    ...(service === "openalex" ? ["DailyQuotaLimited"] : []),
  ];
  for (const health of healths)
    for (const delay of [duration(0), duration(1), duration(7000), maximum]) {
      const operations = [
        { op: "reserve", now: duration(0) },
        { op: "reserve", now: duration(0) },
        {
          op: "finish",
          slot: 0,
          now: duration(10),
          health,
          delay,
          remaining: "50",
          credits: "3",
          reset: duration(10000),
          retry: duration(500),
        },
        {
          op: "finish",
          slot: 1,
          now: duration(11),
          health: "Success",
          remaining: "1000",
        },
        { op: "reserve", now: duration(12), excluded: [1] },
        {
          op: "finish",
          slot: 0,
          now: duration(13),
          health: "Success",
          remaining: "500",
          reset: duration(1),
        },
        { op: "eligible", slot: 0, start: duration(0), now: duration(14) },
        { op: "reserve", now: duration(14) },
        { op: "reserve", now: duration(10010) },
        { op: "reserve", now: maximum },
      ];
      cases.push({
        id: `${service}-health-${health}-${delay.seconds}-${delay.nanoseconds}`,
        service,
        keys: 2,
        process_id: "0",
        process_count: "1",
        capacity: "4",
        epoch: duration(0),
        interval: duration(1100),
        operations,
      });
    }
  for (const epoch of [duration(0), duration("18446744073709551615"), maximum])
    for (const interval of [duration(0), duration(1), maximum]) {
      cases.push({
        id: `${service}-saturation-${epoch.seconds}-${epoch.nanoseconds}-${interval.seconds}`,
        service,
        keys: 3,
        process_id: "4294967296",
        process_count: "18446744073709551615",
        capacity: "18446744073709551615",
        epoch,
        interval,
        operations: [
          { op: "reserve", now: epoch },
          {
            op: "finish",
            slot: 0,
            now: epoch,
            health: "RateLimited",
            delay: maximum,
          },
          { op: "reserve", now: epoch },
          { op: "eligible", slot: 0, start: epoch, now: maximum },
          { op: "reserve", now: maximum },
        ],
      });
    }
}
let state = 0x734819cd;
/** Generate deterministic state transitions and quotas without real credentials. */
function next() {
  state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
  return state;
}
for (const service of ["openalex", "semantic_scholar"])
  for (let sequence = 0; sequence < 80; sequence++) {
    const keys = (next() % 7) + 1;
    const operations = [];
    let now = 0;
    for (let index = 0; index < 80; index++) {
      now += next() % 50;
      const choice = next() % 5;
      if (choice < 2)
        operations.push({
          op: "reserve",
          now: duration(now),
          excluded: [next() % keys, next() % keys],
        });
      else if (choice === 2)
        operations.push({
          op: "eligible",
          now: duration(now),
          slot: next() % (keys + 1),
          start: duration(now - (now % 100)),
        });
      else if (choice === 3 && service === "openalex")
        operations.push({ op: "cancel", slot: next() % (keys + 1) });
      else
        operations.push({
          op: "finish",
          now: duration(now),
          slot: next() % (keys + 1),
          health: [
            "Success",
            "AuthenticationFailure",
            "RateLimited",
            "TransientFailure",
            "TerminalFailure",
            service === "openalex" ? "DailyQuotaLimited" : "Success",
          ][next() % 6],
          delay: duration(next() % 300),
          remaining: next() % 3 ? String(next() % 500) : null,
          credits: next() % 3 ? String(next() % 20) : null,
          reset: next() % 3 ? duration(next() % 300) : null,
          retry: next() % 3 ? duration(next() % 300) : null,
          search: !!(next() % 2),
        });
    }
    cases.push({
      id: `${service}-sequence-${sequence}`,
      service,
      keys,
      process_id: String(next() % 4),
      process_count: String((next() % 4) + 1),
      capacity: String(next() % 33),
      epoch: duration(0),
      interval: duration(1100),
      operations,
    });
  }
const result = spawnSync(
  "output/migration/execution/sources-scheduler-oracle.exe",
  [],
  {
    input: cases.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 96 * 1024 * 1024,
  },
);
assert.equal(result.status, 0, result.stderr || String(result.error));
assert.ifError(result.error);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/scheduler-vectors.json.gz",
  gzipSync(
    JSON.stringify({
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-scheduler-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-schedulers.mjs"),
      ),
      observations,
    }) + "\n",
  ),
);
console.log(`Frozen ${observations.length} original Rust scheduler sequences`);
