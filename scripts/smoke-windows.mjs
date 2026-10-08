/** Verify the extracted Windows distribution without development runtimes on PATH. */
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { inflateSync } from "node:zlib";
import { setTimeout } from "node:timers/promises";

const [archiveArgument, version, commit, mode] = process.argv.slice(2);
assert(mode === undefined || mode === "--local-smoke");
assert.equal(process.platform, "win32");
const archive = path.resolve(archiveArgument);
const report =
  mode === "--local-smoke"
    ? path.join(path.dirname(archive), "smoke")
    : path.resolve("release-results/windows/smoke");
fs.mkdirSync(report, { recursive: true });
const extracted = fs.mkdtempSync(path.join(report, "extracted space-"));
execFileSync("tar", ["-xf", archive, "-C", extracted], { windowsHide: true });
const root = path.join(extracted, `litradar_${version}_windows_amd64`);
const provenance = JSON.parse(
  fs.readFileSync(path.join(root, "build.json"), "utf8"),
);
assert.equal(provenance.sourceCommit, commit);
assert.equal(provenance.version, version);
assert.equal(
  Boolean(provenance.localVerificationOnly),
  mode === "--local-smoke",
);
const popplerBin = path.join(
  root,
  "native",
  provenance.dependencies.poppler.directory,
  "Library/bin",
);
const system = path.join(process.env.SystemRoot, "System32");
const powershell = path.join(system, "WindowsPowerShell/v1.0/powershell.exe");
const environment = { ...process.env };
for (const key of Object.keys(environment)) {
  if (/^(path|LITRADAR_.*|OBSCURA_.*|POPPLER_DATADIR)$/i.test(key))
    delete environment[key];
}
environment.PATH = [root, popplerBin, system, process.env.SystemRoot].join(";");
const launcher = [
  "-NoProfile",
  "-ExecutionPolicy",
  "Bypass",
  "-File",
  path.join(root, "run.ps1"),
];

/** Execute a packaged program with a bounded deadline and the isolated runtime environment. */
function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: extracted,
    env: environment,
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    ...options,
  }).trim();
}
assert.equal(
  run(powershell, [...launcher, "--version"]),
  `litradar ${version}`,
);
const obscura = path.join(root, "obscura.exe");
assert.equal(run(obscura, ["--version"]), "obscura 0.2.4");
const denied = spawnSync(
  obscura,
  ["fetch", "http://127.0.0.1:9/", "--stealth", "--timeout", "5", "--quiet"],
  {
    cwd: extracted,
    env: environment,
    encoding: "utf8",
    windowsHide: true,
    timeout: 15000,
  },
);
assert.ifError(denied.error);
assert.notEqual(denied.status, 0);
assert(
  denied.stderr.includes(
    "Access to private/internal IP address 127.0.0.1 is not allowed",
  ),
);
const pdfTool = path.join(popplerBin, "pdftotext.exe");
run(pdfTool, ["-v"]);

/** Produce a CJK PDF whose extraction requires the distributed character maps. */
function createPdf(text) {
  const encoded = Buffer.from(text, "utf16le").swap16().toString("hex");
  const stream = `BT /F1 12 Tf 72 720 Td <${encoded}> Tj ET\n`;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>",
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream`,
    "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> /FontDescriptor 7 0 R /DW 1000 >>",
    "<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>",
  ];
  let document = "%PDF-1.4\n";
  const offsets = ["0000000000 65535 f \n"];
  for (const [index, object] of objects.entries()) {
    offsets.push(
      `${String(Buffer.byteLength(document)).padStart(10, "0")} 00000 n \n`,
    );
    document += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const start = Buffer.byteLength(document);
  return `${document}xref\n0 ${offsets.length}\n${offsets.join("")}trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${start}\n%%EOF\n`;
}
const pdf = path.join(extracted, "probe.pdf");
const pdfText = "LitRadar 征稿原文";
fs.writeFileSync(pdf, createPdf(pdfText));
assert.equal(
  run(pdfTool, ["-enc", "UTF-8", "-eol", "unix", "-nopgbrk", pdf, "-"]).replace(
    /\s+/gu,
    " ",
  ),
  pdfText,
);

/** Decode real PNG pixel data, ignoring metadata differences. */
function pixels(filename) {
  const png = fs.readFileSync(filename);
  assert(png.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")));
  assert.equal(png.readUInt32BE(16), 32);
  assert.equal(png.readUInt32BE(20), 24);
  const chunks = [];
  for (let offset = 8; offset + 12 <= png.length; ) {
    const length = png.readUInt32BE(offset);
    assert(offset + 12 + length <= png.length);
    if (png.toString("ascii", offset + 4, offset + 8) === "IDAT")
      chunks.push(png.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  assert(chunks.length);
  return inflateSync(Buffer.concat(chunks));
}
const html =
  '<html style="margin:0"><body style="margin:0"><div id="probe" style="width:32px;height:24px;background:red"></div></body></html>';
const frames = [];
for (const color of ["red", "blue"]) {
  const filename = path.join(extracted, `${color}.png`);
  run(
    obscura,
    [
      "fetch",
      "data:text/html," + encodeURIComponent(html),
      "--stealth",
      "--timeout",
      "20",
      "--wait",
      "0",
      "--quiet",
      "--screenshot",
      filename,
      "--eval",
      `document.querySelector('#probe').style.background = '${color}'`,
    ],
    { env: { ...environment, OBSCURA_SHOT_W: "32", OBSCURA_SHOT_H: "24" } },
  );
  frames.push(pixels(filename));
}
assert(
  !frames[0].equals(frames[1]),
  "Rendered pixels did not respond to JavaScript",
);

const previousPath = process.env.PATH;
process.env.PATH = environment.PATH;
try {
  const database = new DatabaseSync(":memory:", { allowExtension: true });
  try {
    database.loadExtension(
      path.join(root, "simple.dll"),
      "sqlite3_simple_init",
    );
    database.exec(
      "CREATE VIRTUAL TABLE articles USING fts5(title, tokenize='simple 0'); INSERT INTO articles VALUES ('科技金融如何赋能企业');",
    );
    assert.equal(
      database
        .prepare(
          "SELECT count(*) AS count FROM articles WHERE articles MATCH simple_query(?)",
        )
        .get("科技金融").count,
      1,
    );
  } finally {
    database.close();
  }
} finally {
  process.env.PATH = previousPath;
}

const reservation = net.createServer();
await new Promise((resolve, reject) => {
  reservation.once("error", reject);
  reservation.listen(0, "127.0.0.1", resolve);
});
const port = reservation.address().port;
await new Promise((resolve) => reservation.close(resolve));
const secret = path.join(extracted, "secret.key");
fs.writeFileSync(secret, randomBytes(32));
const descriptor = fs.openSync(path.join(report, "service.log"), "w");
const service = spawn(
  powershell,
  [
    ...launcher,
    "serve",
    "--host",
    "127.0.0.1",
    "--port",
    String(port),
    "--secret-key-file",
    secret,
  ],
  {
    cwd: extracted,
    env: environment,
    windowsHide: true,
    stdio: ["ignore", descriptor, descriptor],
  },
);
let spawnError;
service.once("error", (error) => {
  spawnError = error;
});
const baseUrl = `http://127.0.0.1:${port}`;
try {
  const deadline = Date.now() + 60000;
  while (true) {
    assert.ifError(spawnError);
    assert.equal(service.exitCode, null, "Service exited before readiness");
    let ready = false;
    try {
      ready = (
        await fetch(`${baseUrl}/health/ready`, {
          signal: AbortSignal.timeout(1000),
        })
      ).ok;
    } catch {}
    if (ready) break;
    assert(Date.now() < deadline, "Windows service readiness timed out");
    await setTimeout(200);
  }
  for (const endpoint of ["/", "/openapi.json"])
    assert((await fetch(baseUrl + endpoint)).ok, endpoint);
  assert(
    !fs.existsSync(path.join(root, "web")),
    "Release must not contain external web assets",
  );
  const home = await fetch(baseUrl + "/");
  assert(home.headers.get("content-security-policy")?.includes("sha256-"));
  assert.equal(home.headers.get("last-modified"), null);
  assert(home.headers.get("etag"));
  const html = await home.text();
  const script = html.match(/src="(\/_next\/static\/[^" ]+\.js)"/);
  assert(script, "Embedded _next script is missing");
  assert(
    (await fetch(baseUrl + script[1])).ok,
    "Embedded script request failed",
  );
  const stylesheet = html.match(/href="(\/_next\/static\/[^" ]+\.css)"/);
  assert(stylesheet, "Embedded _next stylesheet is missing");
  assert((await fetch(baseUrl + stylesheet[1])).ok);
  assert((await fetch(baseUrl + "/login")).ok);
  assert.equal((await fetch(baseUrl + "/missing-embedded-page")).status, 404);
  assert.equal((await fetch(baseUrl + "/api/auth/me")).status, 401);
  assert(fs.existsSync(path.join(root, "data/meta/chinese_journals.csv")));
  const captured = path.join(extracted, "page.json");
  run(
    obscura,
    [
      "fetch",
      baseUrl + "/",
      "--stealth",
      "--timeout",
      "30",
      "--wait-until",
      "domcontentloaded",
      "--wait",
      "0",
      "--eval",
      "JSON.stringify({protocol:'litradar.cfp.page.v1',finalUrl:location.href,html:document.documentElement.outerHTML})",
      "--quiet",
      "--output",
      captured,
    ],
    { env: { ...environment, OBSCURA_ALLOW_PRIVATE_NETWORK: "1" } },
  );
  const page = JSON.parse(fs.readFileSync(captured, "utf8"));
  assert.equal(page.protocol, "litradar.cfp.page.v1");
  assert.equal(page.finalUrl, baseUrl + "/");
  assert(page.html.includes("LitRadar"));
} finally {
  if (service.pid && service.exitCode === null)
    execFileSync(
      path.join(system, "taskkill.exe"),
      ["/PID", String(service.pid), "/T", "/F"],
      { windowsHide: true },
    );
  fs.closeSync(descriptor);
}
fs.writeFileSync(
  path.join(report, "summary.json"),
  JSON.stringify(
    {
      status: "passed",
      ...provenance,
      checks: [
        "relocated-launcher",
        "restricted-path",
        "ready",
        "embedded-web-csp",
        "openapi",
        "anonymous-auth",
        "metadata",
        "native-tokenizer",
        "helper-javascript",
        "private-network-denied",
        "helper-renderer",
        "original-html",
        "cjk-pdf",
      ],
    },
    null,
    2,
  ) + "\n",
);
console.log("Windows archive smoke passed");
