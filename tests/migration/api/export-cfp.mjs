/** Capture original CFP wire responses and authenticated cursors from disposable data. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { setTimeout as delay } from "node:timers/promises";
import { BASELINE, digest, loadOracle } from "../oracle.mjs";

const oracle = await loadOracle(BASELINE);
const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-api-cfp-"));
let child;
let exited;
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
  const base = `http://127.0.0.1:${port}`;
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
        await fetch(base + "/health/ready", {
          signal: AbortSignal.timeout(500),
        })
      ).ok;
    } catch {}
    if (ready) break;
    await delay(100);
  }
  assert(ready, "original API startup timeout");
  const login = await fetch(base + "/api/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username: "fullstack_member",
      password: "FullStackMember!2026",
    }),
    signal: AbortSignal.timeout(10000),
  });
  assert.equal(login.status, 200);
  await login.arrayBuffer();
  const cookie = login.headers.get("set-cookie").split(";")[0];
  const database = new DatabaseSync(path.join(root, "data/auth.sqlite"));
  let snapshot;
  try {
    database.exec("PRAGMA busy_timeout=10000");
    const selected = database
      .prepare(
        "SELECT journal_key,COUNT(*) AS count FROM cfp_notices GROUP BY journal_key HAVING COUNT(*)>=2 ORDER BY COUNT(*),journal_key LIMIT 1",
      )
      .get();
    assert(selected, "missing original CFP seed");
    const row = database
      .prepare("SELECT * FROM cfp_journals WHERE journal_key=?")
      .get(selected.journal_key);
    snapshot = {
      JournalKey: row.journal_key,
      JournalTitle: row.title,
      CheckedOn: row.checked_on,
      SourceUrl: row.source_url,
      SourceStatement: row.source_statement,
      CatalogIds: database
        .prepare(
          "SELECT catalog_id FROM cfp_journal_aliases WHERE journal_key=? ORDER BY catalog_id",
        )
        .all(row.journal_key)
        .map((item) => item.catalog_id),
      Notices: database
        .prepare(
          "SELECT normalized_json FROM cfp_notices WHERE journal_key=? ORDER BY display_order,notice_key",
        )
        .all(row.journal_key)
        .map((item) => JSON.parse(item.normalized_json)),
      Sources: database
        .prepare(
          "SELECT s.source_key AS sourceKey,s.status,s.last_attempt AS lastAttempt,s.last_success AS lastSuccess,s.last_error AS lastError,s.revision,s.lease_expires_at AS leaseExpiresAt FROM cfp_source_journals j JOIN cfp_sources s USING(source_key) WHERE j.journal_key=? ORDER BY s.source_key",
        )
        .all(row.journal_key),
    };
  } finally {
    database.close();
  }
  const columns =
    "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating";
  const rows = [
    [snapshot.CatalogIds[0], "cfp-wire-alias", "Journal <&> Unicode 中文"],
    ["cfp-wire-unadapted", "", "Unadapted"],
  ].map(([id, alias, title]) => {
    const fields = Array(16).fill("");
    fields[0] = id;
    fields[1] = alias;
    fields[2] = title;
    return fields.join(",");
  });
  const catalogCsv = columns + "\n" + rows.join("\n") + "\n";
  await fs.mkdir(path.join(root, "data/meta"), { recursive: true });
  await fs.writeFile(path.join(root, "data/meta/cfp_wire.csv"), catalogCsv);
  /** Capture a response without persisting any session credential. */
  async function read(url, authenticated = true) {
    const response = await fetch(base + url, {
      headers: authenticated ? { cookie } : {},
      signal: AbortSignal.timeout(10000),
    });
    return {
      url,
      authenticated,
      status: response.status,
      body: await response.text(),
    };
  }
  const catalog = await read("/api/cfp/journals?db=cfp_wire.sqlite");
  assert.equal(catalog.status, 200, catalog.body);
  const first = await read(
    "/api/cfp/journals/cfp-wire-alias/notices?db=cfp_wire.sqlite&include_closed=true&limit=1",
  );
  assert.equal(first.status, 200, first.body);
  const cursor = JSON.parse(first.body).page.next_cursor;
  assert(cursor);
  const next = await read(
    `/api/cfp/journals/${snapshot.CatalogIds[0]}/notices?db=cfp_wire.sqlite&include_closed=true&limit=200&cursor=${encodeURIComponent(cursor)}`,
  );
  assert.equal(next.status, 200, next.body);
  const errors = [];
  for (const url of [
    "/api/cfp/journals",
    "/api/cfp/journals?db=x&db=x",
    ...[
      "-1",
      "+",
      "%2B",
      "%2B1",
      "9223372036854775808",
      "18446744073709551616",
      "1.0",
    ].map(
      (limit) =>
        `/api/cfp/journals/x/notices?db=cfp_wire.sqlite&limit=${limit}`,
    ),
  ]) {
    errors.push(await read(url, false));
  }
  await fs.writeFile(
    "tests/migration/api/cfp-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-cfp.mjs"),
        ),
        catalog_csv: catalogCsv,
        snapshot,
        catalog,
        first,
        next,
        errors,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(
    JSON.stringify({
      cfp_original_responses: 3 + errors.length,
      notices: snapshot.Notices.length,
    }),
  );
} finally {
  if (child && child.exitCode === null) child.kill();
  if (exited)
    await Promise.race([
      exited,
      delay(10000, undefined, { ref: false }).then(() => {
        throw new Error("Original API shutdown timeout");
      }),
    ]);
  const resolved = path.resolve(root);
  assert.equal(path.dirname(resolved), path.resolve(os.tmpdir()));
  assert(path.basename(resolved).startsWith("litradar-api-cfp-"));
  await fs.rm(resolved, { recursive: true, force: true });
}
