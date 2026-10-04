/** Export strict wire decoding and continuous-stream framing from the original Rust implementation. */
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
const content = JSON.parse(
  await fs.readFile("tests/migration/index/content-vectors.json", "utf8"),
);
const entry = content.observations[0].input.operations[0].catalog;
const batch = content.observations[0].input.operations[0].batch;
const assignment = {
  journal_ordinal: 2,
  entry,
  mode: "incremental",
  committed_anchor: null,
  traversal_checkpoint: "opaque",
};
const config = {
  timeout_seconds: 30,
  openalex_api_keys: ["key"],
  semantic_scholar_api_keys: [],
  crossref_mailtos: [],
  semantic_scholar_worker_id: 0,
  semantic_scholar_process_count: 1,
  semantic_scholar_base_interval_ms: 3000,
  schedule_epoch_unix_millis: 42,
};
const failure = {
  class: "provider",
  operation: "provider_request",
  sqlite_code: null,
  sqlite_extended_code: null,
  is_busy_or_locked: false,
};
const base = {
  request: {
    protocol_version: 8,
    catalog_name: "example",
    provider_name: "scholarly",
    run_id: "run",
    worker_id: 0,
    process_count: 1,
    source_worker_count: 2,
    schedule_epoch_unix_millis: 42,
    timeout_seconds: 30,
    assignments: [assignment],
  },
  assignment,
  bootstrap: {
    protocol_version: 8,
    worker_id: 0,
    cnki_captcha_token: null,
    provider_proxy_url: null,
    scholarly_config: config,
    scholarly_workset_dir: "C:/private/workset",
  },
  worker: {
    type: "batch",
    protocol_version: 8,
    worker_id: 0,
    sequence: 0,
    journal_ordinal: 2,
    page_index: 0,
    batch,
  },
  parent: {
    type: "committed",
    protocol_version: 8,
    worker_id: 0,
    sequence: 0,
    journal_ordinal: 2,
    page_index: 0,
    is_complete: true,
  },
  failure,
};
const requests = [];
/** Add an exact JSON spelling without losing duplicates or numeric forms. */
function add(kind, name, payload, stream = false, reads = 3) {
  requests.push({
    op: "worker_wire",
    kind,
    name,
    payload: typeof payload === "string" ? payload : JSON.stringify(payload),
    stream,
    reads,
  });
}
for (const [kind, value] of Object.entries(base)) {
  add(kind, "valid", value);
  add(kind, "sequence", Object.values(value));
  add(kind, "extra-sequence", [...Object.values(value), null]);
  add(kind, "null", "null");
  add(kind, "unknown", { ...value, unexpected: 1 });
  const text = JSON.stringify(value);
  add(
    kind,
    "duplicate",
    text.slice(0, -1) +
      `,"${Object.keys(value)[0]}":${JSON.stringify(Object.values(value)[0])}}`,
  );
  for (const key of Object.keys(value)) {
    const copy = { ...value };
    delete copy[key];
    add(kind, `missing-${key}`, copy);
    add(kind, `null-${key}`, { ...value, [key]: null });
  }
  for (const [key, field] of Object.entries(value)) {
    if (typeof field === "number") {
      for (const numeric of [
        "-1",
        "1.0",
        "1e0",
        '"1"',
        "18446744073709551616",
        "4294967296",
      ]) {
        add(
          kind,
          `number-${key}-${numeric}`,
          text.replace(`"${key}":${field}`, `"${key}":${numeric}`),
        );
      }
    }
  }
  add(kind, "trailing-value", text + "{}");
  add(kind, "trailing-space", text + " \n\t");
  add(kind, "stream-consecutive", text + text + text, true);
  add(kind, "stream-truncated", text + text.slice(0, -2), true, 2);
  add(kind, "stream-whitespace", text + " \n", true, 2);
  add(kind, "stream-empty", "", true, 1);
  add(kind, "stream-invalid", "!", true, 1);
}
add("bootstrap", "defaulted-sequence", [8, 0]);
add("bootstrap", "partial-sequence", [8, 0, "token"]);
add("bootstrap", "unknown-nested-config", {
  ...base.bootstrap,
  scholarly_config: { ...config, extra: 1 },
});
add("assignment", "object-unit-mode", {
  ...assignment,
  mode: { incremental: null },
});
add("failure", "object-unit-enums", {
  ...failure,
  class: { provider: null },
  operation: { provider_request: null },
});
for (const type of ["succeeded", "failed"]) {
  const value = {
    type,
    protocol_version: 8,
    worker_id: 0,
    sequence: 2,
    ...(type === "failed" ? { failure } : {}),
  };
  add("worker", type, value);
  add("worker", `${type}-sequence`, Object.values(value));
  add("worker", `${type}-cross-fields`, { ...value, batch });
}
add(
  "worker",
  "large-unlimited-stream",
  {
    ...base.worker,
    batch: {
      ...batch,
      journal: { ...batch.journal, observed_title: "x".repeat(131072) },
    },
  },
  true,
  1,
);
const observations = [];
for (const input of requests) {
  const result = spawnSync(build.binary, [], {
    input: JSON.stringify(input) + "\n",
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    maxBuffer: 16 * 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  observations.push(JSON.parse(result.stdout));
}
assert(
  observations.find(
    (value) => value.input.kind === "parent" && value.input.name === "sequence",
  ).expected.value,
);
assert(
  observations.find((value) => value.input.name === "unknown-nested-config")
    .expected.value,
);
await fs.writeFile(
  "tests/migration/index/worker-vectors.json",
  JSON.stringify(
    {
      build,
      exporter_sha256: digest(
        await fs.readFile("tests/migration/index/export-worker.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust worker wire observations`,
);
