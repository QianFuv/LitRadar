/** Freeze original collection, replay, grouping and emission transitions on disposable files. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const initial = {
  token: "0123456789abcdef0123456789abcdef",
  issn: "1234-5679",
  frozen_at: 2,
  created_from: null,
  updated_from: null,
  root_total: null,
  candidate: null,
  generation: 0,
  sequence: 0,
  phase: { kind: "discover" },
};
/** Build a deterministic record with meaningful issue identity. */
function work(index, second = 1, issue = "1", date = [2026, 1, 1]) {
  return {
    DOI: `10.1234/${index}`,
    title: [`Work ${index}`],
    created: { timestamp: second * 1000 },
    published: { "date-parts": [date] },
    volume: "1",
    issue,
  };
}
/** Express one HTTP observation consumed by the original state machine. */
function page(start, count, total, second = 1) {
  return {
    kind: "accept",
    page: {
      items: Array.from({ length: count }, (_, offset) =>
        work(start + offset, second),
      ),
      total_results: total,
      next_cursor: String(start + count),
    },
  };
}
const cases = [];
/** Record a complete isolated workflow with stable operation IDs. */
function add(name, actions, state = initial) {
  cases.push({
    id: name,
    kind: "workflow",
    input: JSON.stringify({ initial: state, actions }),
  });
}
const groups = { kind: "groups" };
const emit = { kind: "emit", upper: "9999-12-31" };
const seal = { kind: "seal", upper: "9999-12-31" };
add("empty", [page(0, 0, 0), groups, seal, emit]);
add("singleton-replay", [
  page(0, 1, 1),
  { kind: "reopen", index: 0 },
  groups,
  seal,
  { kind: "reopen", index: 3 },
  emit,
  emit,
]);
add("filtered-singleton", [page(0, 1, 1), page(0, 1, 1), groups, emit], {
  ...initial,
  updated_from: "1970-01-01",
});
add("discovery-errors", [
  page(0, 0, 1),
  page(0, 1, 0),
  page(0, 2, 2),
  page(0, 1, 1, 3),
]);
for (const count of [2, 224, 225])
  add(`leaf-${count}`, [
    page(0, 1, count),
    page(0, count, count),
    groups,
    seal,
    emit,
  ]);
for (const tail of [0, 1])
  add(
    `dense-${tail}`,
    [
      page(0, 1, 450),
      page(0, 225, 450),
      page(0, 225, 450),
      page(225, 225, 450),
      { kind: "reopen", index: 4 },
      page(450, tail, 450),
      groups,
      emit,
    ],
    { ...initial, frozen_at: 1 },
  );
add(
  "dense-repeated-key",
  [
    page(0, 1, 450),
    page(0, 225, 450),
    page(0, 225, 450),
    page(0, 225, 450),
    page(0, 225, 450),
    page(0, 225, 450),
  ],
  { ...initial, frozen_at: 1 },
);
add("leaf-truncation", [page(0, 1, 2), page(0, 1, 2), page(0, 1, 2)]);
add("leaf-duplicate", [
  page(0, 1, 2),
  {
    kind: "accept",
    page: { items: [work(0), work(0)], total_results: 2, next_cursor: null },
  },
  page(0, 2, 2),
]);
add("count-generation", [
  page(0, 1, 2),
  page(0, 1, 2),
  page(0, 1, 3),
  page(0, 1, 3),
  page(0, 1, 4),
]);
add("partitioned", [
  page(0, 1, 226),
  page(0, 225, 226),
  page(0, 113, 113),
  page(113, 113, 113, 2),
  groups,
  emit,
]);
add("partition-drift", [
  page(0, 1, 226),
  page(0, 225, 226),
  page(0, 113, 113),
  page(113, 112, 112, 2),
]);
add("cross-leaf-duplicate", [
  page(0, 1, 226),
  page(0, 225, 226),
  page(0, 113, 113),
  page(0, 113, 113, 2),
]);
add("whole-group-window", [
  page(0, 1, 3),
  {
    kind: "accept",
    page: {
      items: [
        work(0, 1, "1", [2026, 2, 1]),
        work(1),
        work(2, 1, "2", [2026, 1, 31]),
      ],
      total_results: 3,
      next_cursor: null,
    },
  },
  groups,
  { kind: "emit", upper: "2026-02-01", lower: "2026-02-01" },
  {
    kind: "emit",
    upper: "9999-12-31",
    after: { rank: 2, date: "2026-01-31", key: "doi:10.1234/2" },
  },
  {
    kind: "emit",
    upper: "9999-12-31",
    after: { rank: 1, date: "2026-01-31", key: "doi:10.1234/2" },
  },
]);
add("unknown", [
  {
    kind: "accept",
    page: {
      items: [{ created: { timestamp: 1000 } }],
      total_results: 1,
      next_cursor: null,
    },
  },
  groups,
  emit,
]);
add("invalid-seal", [
  page(0, 1, 1),
  { kind: "seal", upper: "bad" },
  seal,
  seal,
  page(0, 1, 1),
]);
const directory = await fs.mkdtemp(
  path.resolve("output/migration/execution/workset-flows-"),
);
const requests = cases.map((value, index) => ({
  ...value,
  root: path.join(directory, String(index)),
}));
const result = spawnSync(
  "output/migration/execution/sources-workset-oracle.exe",
  [],
  {
    input: requests.map((value) => JSON.stringify(value)).join("\n") + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 120000,
    maxBuffer: 64 * 1024 * 1024,
  },
);
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => {
    const value = JSON.parse(line);
    delete value.root;
    return value;
  });
assert.equal(observations.length, cases.length);
await fs.writeFile(
  "tests/migration/sources/workset-flow-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-workset-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-workset-flows.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust durable workset workflows`,
);
