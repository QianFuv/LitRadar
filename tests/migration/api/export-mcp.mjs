/** Capture original executable MCP argument behavior using disposable synthetic credentials. */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { DatabaseSync } from "node:sqlite";
import { BASELINE, digest, loadOracle } from "../oracle.mjs";

const oracle = await loadOracle(BASELINE);
const root = await fs.mkdtemp(path.join(os.tmpdir(), "litradar-api-mcp-"));
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
  const seeded = spawnSync(
    path.join(oracle.directory, oracle.manifest.binaries.fullStackFixture),
    ["--project-root", root],
    {
      cwd: root,
      timeout: 30_000,
      encoding: "utf8",
      maxBuffer: 1024 * 1024,
      windowsHide: true,
    },
  );
  assert.ifError(seeded.error);
  assert.equal(seeded.status, 0, seeded.stderr);
  const fixtures = [];
  await fs.mkdir("tests/migration/api/fixtures", { recursive: true });
  for (const name of (await fs.readdir(path.join(root, "data/index")))
    .filter((name) => name.endsWith(".sqlite"))
    .sort()) {
    const data = await fs.readFile(path.join(root, "data/index", name));
    const target = `tests/migration/api/fixtures/${name}.fixture`;
    await fs.writeFile(target, data);
    fixtures.push({ name, path: target, sha256: digest(data) });
  }
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
  const deadline = Date.now() + 30_000;
  let ready = false;
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null);
    try {
      ready = (
        await fetch(`${base}/health/ready`, {
          signal: AbortSignal.timeout(500),
        })
      ).ok;
    } catch {}
    if (ready) break;
    await delay(100);
  }
  assert(ready, "Original API startup exceeded 30 seconds");
  const login = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username: "fullstack_member",
      password: "FullStackMember!2026",
    }),
    signal: AbortSignal.timeout(10_000),
  });
  assert.equal(login.status, 200);
  await login.arrayBuffer();
  const cookie = login.headers
    .getSetCookie()
    .find((value) => value.startsWith("litradar_session="))
    .split(";")[0];
  let session;
  /** Send one bounded original MCP request without persisting credentials or sessions. */
  async function post(body) {
    const headers = {
      cookie,
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
    };
    if (session)
      Object.assign(headers, {
        "mcp-session-id": session,
        "mcp-protocol-version": "2025-06-18",
      });
    const response = await fetch(`${base}/mcp`, {
      method: "POST",
      headers,
      body,
      signal: AbortSignal.timeout(10_000),
    });
    const text = await response.text();
    assert.equal(response.status, 200, text);
    session ??= response.headers.get("mcp-session-id");
    const line = text.split("\n").find((value) => value.startsWith("data: {"));
    assert(line, "Missing original MCP JSON event");
    return JSON.parse(line.slice(6));
  }
  await post(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "migration-observer", version: "1" },
      },
    }),
  );
  const inputs = [];
  const floatCases = [];
  const database = new DatabaseSync(path.join(root, "data/auth.sqlite"));
  try {
    for (const stamp of [1, 0.000001, 10000000000000000]) {
      database
        .prepare(
          "UPDATE folders SET created_at=? WHERE user_id=(SELECT id FROM users WHERE username='fullstack_member')",
        )
        .run(stamp);
      const response = await post(
        JSON.stringify({
          jsonrpc: "2.0",
          id: 3,
          method: "tools/call",
          params: { name: "list_folders", arguments: {} },
        }),
      );
      const text = response.result.content[0].text;
      assert(JSON.parse(text).length > 0, "missing controlled folder fixture");
      floatCases.push({ folders: JSON.parse(text), text });
    }
  } finally {
    database.close();
  }
  await fs.writeFile(
    "tests/migration/api/mcp-float-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-mcp.mjs"),
        ),
        cases: floatCases,
      },
      null,
      2,
    ) + "\n",
  );
  for (const raw of [
    "{}",
    '{"article_id":null}',
    '{"article_id":1}',
    '{"article_id":1.0}',
    '{"article_id":true}',
    '{"article_id":[]}',
    '{"article_id":{}}',
    '{"article_id":""}',
    '{"article_id":"  "}',
    '{"article_id":"+1","db":" "}',
    '{"article_id":"9223372036854775808"}',
  ])
    inputs.push(["get_article", raw]);
  for (const raw of [
    "{}",
    '{"folder_id":0,"article_id":"bad"}',
    '{"folder_id":1.0,"article_id":"1"}',
    '{"folder_id":9223372036854775808,"article_id":"1"}',
    '{"folder_id":18446744073709551616,"article_id":"1"}',
    '{"folder_id":-9223372036854775809,"article_id":"1"}',
    '{"folder_id":1e3,"article_id":"1"}',
    '{"folder_id":1,"article_id":"1","db_name":" "}',
  ])
    inputs.push(["add_favorite", raw]);
  for (const field of [
    "area",
    "journal_id",
    "abs_rating",
    "year",
    "limit",
    "include_total",
    "q",
  ]) {
    for (const value of [null, true, 1.5, {}, [null]])
      inputs.push([
        "search_articles",
        JSON.stringify({ [field]: value, db: " " }),
      ]);
  }
  for (const args of [
    { journal_id: ["bad"], area: [""], db: " " },
    { journal_id: Array(501).fill(""), db: " " },
    { area: [""], issue_id: -1, db: " " },
    { issue_id: -1, year: -1 },
    { year: -1, q: " " },
    { search_mode: "RAW", limit: 0 },
    { search_mode: "SİMPLE", limit: 0 },
    { search_mode: " ADVANCED ", limit: 201 },
    { limit: 0, offset: -1 },
    { offset: -1, cursor: " " },
    { cursor: " ", db: " " },
    { area: "中".repeat(2049), db: " " },
  ])
    inputs.push(["search_articles", JSON.stringify(args)]);
  for (const args of [
    { db: " ", area: " " },
    { year: -1, sort: " " },
    { sort: " ", limit: 0 },
    { limit: 0, offset: -1 },
  ])
    inputs.push(["list_journals", JSON.stringify(args)]);
  const cases = [];
  for (const limit of [
    "\u0000",
    "\u0007",
    "\u0301",
    "\u007f",
    '"',
    "\u2028",
    "\\",
  ])
    inputs.push(["list_journals", JSON.stringify({ limit })]);
  for (const [name, argumentsJson] of inputs) {
    const response = await post(
      `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":${JSON.stringify(name)},"arguments":${argumentsJson}}}`,
    );
    assert(
      response.result?.isError,
      `Expected a bounded validation failure: ${name} ${argumentsJson}`,
    );
    cases.push({
      name,
      arguments_json: argumentsJson,
      result: response.result,
    });
  }
  const filename = "tests/migration/api/mcp-vectors.json";
  await fs.writeFile(
    filename,
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-mcp.mjs"),
        ),
        cases,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify({ output: filename, cases: cases.length }));
  const httpCases = [];
  for (const url of [
    "/api/journals?has_articles=YES",
    "/api/journals?has_articles=1",
    "/api/journals?has_articles=",
    "/api/journals?year=",
    "/api/journals?year=1.0",
    "/api/journals?year=%201%20",
    "/api/journals?year=1&year=2",
    "/api/journals?db=x&db=y",
    "/api/journals?unknown=1&unknown=2",
    "/api/journals?db=%FF",
    "/api/journals?db=%",
    "/api/journals?year=9223372036854775808",
    "/api/journals?year=-9223372036854775809",
    "/api/journals?year=%2B1",
    "/api/journals/not-an-integer",
    "/api/journals/9223372036854775808",
    "/api/journals/%201%20",
    "/api/journals/%FF",
    "/api/meta/areas?db=x&db=y",
    "/api/articles?journal_id=no",
    "/api/articles?q=%FF",
    "/api/articles?q=%",
    "/api/weekly-updates/articles?journal_id=no",
    "/api/issues?journal_id=no",
    "/api/issues?sort=%FF%FF",
    "/api/issues?sort=%E2%82",
    "/api/issues?sort=x%E2%80%A8y",
    "/api/issues?sort=x%E2%80%A9y",
  ]) {
    for (const authenticated of [false, true]) {
      const response = await fetch(base + url, {
        headers: authenticated ? { cookie } : {},
        signal: AbortSignal.timeout(10_000),
      });
      httpCases.push({
        url,
        authenticated,
        status: response.status,
        content_type: response.headers.get("content-type"),
        body: await response.text(),
      });
    }
  }
  await fs.writeFile(
    "tests/migration/api/http-query-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        fixtures,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-mcp.mjs"),
        ),
        cases: httpCases,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify({ http_query_cases: httpCases.length }));
  const jsonCases = [];
  const favoriteCases = [];
  for (const [method, url, body] of [
    ["GET", "/api/favorites/check", ""],
    ["GET", "/api/favorites/check?article_id=0", ""],
    ["GET", "/api/favorites/check?article_id=9007199254740993", ""],
    ["GET", "/api/favorites/check?article_id=x", ""],
    ["GET", "/api/favorites/folders/99999/articles", ""],
    ["GET", "/api/favorites/folders/99999/articles?limit=0", ""],
    ["GET", "/api/favorites/folders/99999/articles?offset=-1", ""],
    ["GET", "/api/favorites/folders/99999/articles/page?limit=501", ""],
    ["GET", "/api/favorites/folders/99999/articles/page?cursor=x", ""],
    ["GET", "/api/favorites/folders/-1/articles/page?limit=0", ""],
    ["GET", "/api/favorites/folders/99999/count", ""],
    ["GET", "/api/favorites/folders/99999/export?format=RIS", ""],
    ["GET", "/api/favorites/folders/99999/export", ""],
    ["DELETE", "/api/favorites/folders/99999/articles/no", ""],
    ["DELETE", "/api/favorites/folders/no/articles/1", ""],
    ["DELETE", "/api/favorites/folders/99999/articles/1", ""],
    ["POST", "/api/favorites/folders/99999/articles", '{"article_id":1}'],
    ["POST", "/api/favorites/folders", '{"name":""}'],
    ["PUT", "/api/favorites/folders/99999", '{"name":"x"}'],
    ["DELETE", "/api/favorites/folders/99999", ""],
    ["POST", "/api/favorites/folders/99999/articles/bulk", '{"articles":[]}'],
    [
      "POST",
      "/api/favorites/check/batch",
      '{"article_ids":[0,-1,9007199254740993]}',
    ],
  ]) {
    for (const authenticated of [false, true]) {
      const response = await fetch(base + url, {
        method,
        headers: {
          "content-type": "application/json",
          ...(authenticated ? { cookie } : {}),
        },
        ...(method === "GET" ? {} : { body }),
        signal: AbortSignal.timeout(10_000),
      });
      favoriteCases.push({
        method,
        url,
        body,
        authenticated,
        status: response.status,
        content_type: response.headers.get("content-type"),
        response: await response.text(),
      });
    }
  }
  await fs.writeFile(
    "tests/migration/api/http-favorite-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-mcp.mjs"),
        ),
        cases: favoriteCases,
      },
      null,
      2,
    ) + "\n",
  );
  for (const [type, url, method, bodies] of [
    [
      "ScheduledTaskCreate",
      "/api/admin/scheduled-tasks",
      "POST",
      [
        "{}",
        "[]",
        '{"unknown":1}',
        ...[
          "null",
          "false",
          "0",
          '"index"',
          "{}",
          "[]",
          '["index"]',
          '["index",null,true,false]',
          '["index","catalog.csv",true]',
          '["index",null,true,false,0]',
          '["notify"]',
          '["notify",null,12]',
          '["push","catalog.sqlite"]',
          '["index",{"notify":true}]',
          "[null]",
          '{"kind":"index"}',
          '{"kind":"notify","max_candidates":18446744073709551615}',
          '{"kind":"index","notify":null}',
          '{"kind":"index","notify":"bad"}',
          '{"notify":"bad","kind":"index"}',
          '{"notify":"bad","kind":"shell"}',
          '{"kind":"index","notify":"bad","later":BROKEN}',
          '{"kind":"index","notify":false,"notify":true,"kind":"push"}',
          '{"kind":0}',
          '{"kind":"Index"}',
          '{"kind":"index","database":"a"}',
          '{"anything":1e9999}',
          '{"kind":"index","anything":"\\uD800"}',
          '{"kind":"notify","max_candidates":-1}',
          '{"kind":"notify","max_candidates":1.0}',
        ].map((job) => `{"name":"x","cron":"* * * * *","job":${job}}`),
        '{"name":"x","job":{"kind":"index","notify":"bad"},"cron":"* * * * *"}',
        '["x",{"kind":"index","notify":"bad"},"* * * * *"]',
        '{"name":"x","cron":"* * * * *","job":{"kind":"index"},"timeout_seconds":-1}',
      ],
    ],
    [
      "ScheduledTaskUpdate",
      "/api/admin/scheduled-tasks/1",
      "PUT",
      [
        "{}",
        "[]",
        "[null,null,null,null,null,null,null]",
        '{"job":null}',
        '{"job":{"kind":"push"}}',
        '{"job":{"kind":"index","notify":"bad"}   }',
        '{"job":{"kind":"index","notify":"bad"}   ,"enabled":true}',
      ],
    ],
    [
      "RuntimeSettingsUpdate",
      "/api/admin/runtime-settings",
      "PUT",
      [
        "{}",
        "[]",
        '{"values":null}',
        '{"values":{"log_format":null}}',
        '{"values":{"a":"x","a":"y"}}',
        '{"values":{"a":false}}',
        '{"secret_pool_updates":{"x":{}}}',
        '{"secret_pool_updates":{"x":[]}}',
        '{"secret_pool_updates":{"x":{"add":null}}}',
        '{"secret_pool_updates":{"x":{"unknown":true}}}',
      ],
    ],
    [
      "NotificationSettingsUpdate",
      "/api/tracking/notification-settings",
      "PUT",
      [
        "{}",
        "[]",
        "[[]]",
        '[[],[],[],"pushplus",null]',
        "null",
        '{"keywords":null}',
        '{"keywords":[null]}',
        '{"enabled":null}',
        '{"pushplus_token":null}',
        '{"pushplus_token":""}',
        '{"pushplus_token":false}',
        '{"pushplus_token":null,"pushplus_token":"x"}',
        '{"ai_api_key":{}}',
        '{"ai_backup_api_key":null}',
        '{"ai_retry_attempts":1.0}',
        '{"selected_databases":[1]}',
      ],
    ],
    [
      "LoginRequest",
      "/api/auth/login",
      "POST",
      [
        "{}",
        "[]",
        "[1]",
        '["u"]',
        '["u","p",1]',
        "[",
        '["u","p" false]',
        '{"username":1,"password":BROKEN}',
        '{"username":"u","username" BROKEN}',
        '{"Username":"u","password":"p"}',
        '{"username":null}',
        '{"username":"\\uD800"}',
        '{"username":"中文","password":false}',
        '{"username":"u"} true',
      ],
    ],
    [
      "TokenCreateRequest",
      "/api/auth/tokens",
      "POST",
      [
        "{}",
        "[]",
        '["x"]',
        '["x",3600]',
        '["x",3600,1]',
        '{"ttl":null}',
        '{"ttl":-0}',
        '{"ttl":1.0}',
        '{"ttl":1e9999}',
        '{"ttl":9223372036854775808}',
        '{"ttl":"42"}',
        '{"name":true}',
        '{"unknown":1e9999}',
        '{"unknown":"\\uD800"}',
        '{"unknown":' + "[".repeat(140) + "0" + "]".repeat(140) + "}",
        '{"unknown":1,"unknown":2}',
        '{"name":"x","name" BROKEN}',
        '{"name":"x","name"}',
        "{}\u0000garbage",
        "\u0000",
        '{"name":\u0000}',
        '{"unknown":BROKEN}',
        '{"unknown":[0,]}',
        '{"unknown":{"a":1,}}',
        '{"name":"x",',
        '{"ttl":1.7976931348623158e308}',
        "{} true",
      ],
    ],
    [
      "FolderCreate",
      "/api/favorites/folders",
      "POST",
      [
        "{}",
        "[]",
        '["x"]',
        '["x",true]',
        '{"name":"x","is_tracking":null}',
        '{"name":"x","is_tracking":1}',
        '{"name":[]}',
        '{"name":{}}',
      ],
    ],
    [
      "FavoriteAdd",
      "/api/favorites/folders/1/articles",
      "POST",
      [
        "{}",
        "[]",
        '["+42"]',
        '{"article_id":null}',
        '{"article_id":-0}',
        '{"article_id":42.0}',
        '{"article_id":"+42"}',
        '{"article_id":"0042"}',
        '{"article_id":" 42"}',
        '{"article_id":"9223372036854775808"}',
        '{"article_id":18446744073709551615}',
        '{"article_id":42,"note":null}',
      ],
    ],
    [
      "FavoriteBulkAdd",
      "/api/favorites/folders/1/articles/bulk",
      "POST",
      [
        '{"articles":[{"article_id":false}]}',
        '{"articles":[',
        '{"articles":[{}]}',
        '{"articles":null}',
        '{"articles":[{"article_id":1,"article_id":2}]}',
      ],
    ],
    [
      "AdminInviteCodeCreate",
      "/api/admin/invite-codes",
      "POST",
      ["", "null", "{}", "[]", "[null,null]", '{"unknown":1}'],
    ],
  ]) {
    for (const body of bodies) {
      const response = await fetch(base + url, {
        method,
        headers: { "content-type": "application/json" },
        body,
        signal: AbortSignal.timeout(10_000),
      });
      jsonCases.push({
        type,
        body,
        content_type: "application/json",
        status: response.status,
        response: await response.text(),
      });
    }
  }
  for (const contentType of [
    null,
    "",
    "text/json",
    "APPLICATION/JSON",
    "application/problem+json",
    "application/json; charset=latin1",
    "application/json;broken",
    "application/+json",
    "application/json; charset=a; charset=b",
    "application/json ; charset=utf-8",
  ]) {
    const response = await fetch(base + "/api/auth/tokens", {
      method: "POST",
      headers: contentType === null ? {} : { "content-type": contentType },
      body: Buffer.from("{}"),
      signal: AbortSignal.timeout(10_000),
    });
    jsonCases.push({
      type: "TokenCreateRequest",
      body: "{}",
      content_type: contentType,
      status: response.status,
      response: await response.text(),
    });
  }
  await fs.writeFile(
    "tests/migration/api/http-json-vectors.json",
    JSON.stringify(
      {
        baseline: BASELINE,
        exporter_sha256: digest(
          await fs.readFile("tests/migration/api/export-mcp.mjs"),
        ),
        cases: jsonCases,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify({ http_json_cases: jsonCases.length }));
} finally {
  if (child && child.exitCode === null) child.kill();
  if (exited)
    await Promise.race([
      exited,
      delay(10_000, undefined, { ref: false }).then(() => {
        throw new Error("Original API shutdown exceeded 10 seconds");
      }),
    ]);
  const resolved = path.resolve(root);
  assert.equal(path.dirname(resolved), path.resolve(os.tmpdir()));
  assert(path.basename(resolved).startsWith("litradar-api-mcp-"));
  await fs.rm(resolved, { recursive: true, force: true });
}
