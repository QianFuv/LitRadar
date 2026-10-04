/** Audit actual production artifacts and apply only explicitly approved bounded exceptions. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { verifyDependency } from "./dependency.mjs";

const directory = path.resolve("output/security");
const tools = path.join(directory, "tools");
const suffix = process.platform === "win32" ? ".exe" : "";
const syftImage =
  "anchore/syft@sha256:0356562f495d432056237fbea5cbc2d4839c9c75cd500784a66de2e7cc95ca7c";

/** Hash exact artifact bytes for the verification record. */
function digest(filename) {
  return createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
}

/** Require every packaged source and embedded asset exactly once, with matching bytes. */
export function verifyImageSources(content, roots) {
  const lines = content.trim().split(/\r?\n/);
  const expected = roots
    .flatMap((root) =>
      fs
        .readdirSync(root, { recursive: true, withFileTypes: true })
        .filter((entry) => entry.isFile())
        .map((entry) =>
          path
            .relative(process.cwd(), path.join(entry.parentPath, entry.name))
            .replaceAll("\\", "/"),
        ),
    )
    .sort();
  const actual = [];
  for (const line of lines) {
    const match = line.match(
      /^([a-f0-9]{64})  ((?:third_party|cmd|internal|assets)\/[^\\]+)$/,
    );
    assert(
      match && !match[2].split("/").includes(".."),
      "Invalid image source inventory",
    );
    actual.push(match[2]);
    assert.equal(
      digest(match[2]),
      match[1],
      "Image source differs: " + match[2],
    );
  }
  assert(expected.length > 0, "Empty image source set");
  assert.deepEqual(actual.sort(), expected, "Image source file set differs");
}

/** Run every local release scanner against the same final image inventories. */
export async function runSecurity() {
  fs.mkdirSync(directory, { recursive: true });
  const report = {
    status: "In Progress",
    started: new Date().toISOString(),
    commands: [],
    exceptions: [],
    images: [],
    waivedChecks: [
      {
        scope: "Obscura supply chain",
        approval: "A13: explicit user instruction on 2026-10-05",
        status: "Waived by user; not scanned or certified",
      },
    ],
    remoteActions: "Not Run",
    remoteCodeQL: "Not Run",
  };
  const policy = JSON.parse(
    fs.readFileSync("scripts/security-exceptions.json", "utf8"),
  );
  const pins = JSON.parse(
    fs.readFileSync("scripts/security-tools.json", "utf8"),
  );
  const environment = {
    ...process.env,
    GOWORK: "off",
    GOENV: "off",
    GOFLAGS: "",
    GOTOOLCHAIN: "go1.27.1",
    CGO_ENABLED: "1",
  };
  /** Preserve exact commands and failed raw scanner outputs before interpreting approved exceptions. */
  function execute(
    id,
    executable,
    args,
    allowFindings = false,
    timeout = 600000,
    input,
  ) {
    const started = new Date().toISOString();
    const result = spawnSync(executable, args, {
      env: environment,
      input,
      encoding: "utf8",
      timeout,
      windowsHide: true,
      maxBuffer: 128 * 1024 * 1024,
    });
    const filename = path.join(directory, `${id}.log`);
    fs.writeFileSync(
      filename,
      (id === "working-diff"
        ? "Working diff retained only in memory for secret scanning"
        : (result.stdout ?? "")) +
        "\n" +
        (result.stderr ?? ""),
    );
    report.commands.push({
      id,
      executable,
      args,
      started,
      finished: new Date().toISOString(),
      exitCode: result.status,
      error: result.error?.message,
      log: filename,
      sha256: digest(filename),
    });
    fs.writeFileSync(
      path.join(directory, "result.json"),
      JSON.stringify(report, null, 2) + "\n",
    );
    assert.ifError(result.error);
    assert(
      allowFindings ? [0, 1].includes(result.status) : result.status === 0,
      `${id} failed; inspect ${filename}`,
    );
    return result.stdout;
  }
  /** Accept only the exact advisory, ecosystem and package version authorized by A10. */
  function accept(id, name, version, ecosystem) {
    const approved = policy.exceptions.find(
      (entry) =>
        entry.id === id &&
        entry.name === name &&
        entry.version === version &&
        entry.ecosystem === ecosystem,
    );
    assert(approved, `Unapproved security finding: ${id} ${name}@${version}`);
    report.exceptions.push({
      ...approved,
      owner: policy.owner,
      reviewBy: policy.reviewBy,
      approval: policy.approval,
    });
  }
  try {
    assert(
      new Date().toISOString().slice(0, 10) <= policy.reviewBy,
      "Security exception review expired",
    );
    for (const tool of pins.tools.filter((entry) => entry.name !== "syft")) {
      const argumentsList =
        tool.name === "gitleaks"
          ? ["version"]
          : tool.name === "actionlint"
            ? ["-version"]
            : ["--version"];
      const output = execute(
        `version-${tool.name}`,
        path.join(tools, tool.name + suffix),
        argumentsList,
      );
      assert(output.includes(tool.version), `Incorrect ${tool.name} version`);
    }
    assert(
      execute("version-govulncheck", path.join(tools, "govulncheck" + suffix), [
        "-version",
      ]).includes(pins.govulncheck),
    );
    execute("security-policy-tests", process.execPath, [
      "--test",
      "tests/migration/security.test.mjs",
      "tests/profiling/go-image.test.mjs",
    ]);
    await verifyDependency("go-sdk");
    await verifyDependency("go-sqlite3");
    execute("actionlint", path.join(tools, "actionlint" + suffix), []);
    for (const filename of fs
      .readdirSync(".github/workflows")
      .filter((name) => /\.ya?ml$/.test(name))) {
      for (const line of fs
        .readFileSync(`.github/workflows/${filename}`, "utf8")
        .split(/\r?\n/)) {
        const reference = line.match(/^\s*(?:-\s*)?uses:\s+(\S+)/)?.[1];
        if (reference && !reference.startsWith("./"))
          assert(
            /@[a-f0-9]{40}$/.test(reference) && /# v\d/.test(line),
            `Unpinned action in ${filename}: ${reference}`,
          );
      }
    }
    for (const match of fs
      .readFileSync(".gitleaksignore", "utf8")
      .matchAll(/^# Review by: (\d{4}-\d{2}-\d{2})/gm))
      assert(
        new Date().toISOString().slice(0, 10) <= match[1],
        "Secret exception review expired",
      );
    execute("gitleaks", path.join(tools, "gitleaks" + suffix), [
      "git",
      "--redact",
      "--report-format",
      "sarif",
      "--report-path",
      path.join(directory, "gitleaks.sarif"),
      "--exit-code",
      "1",
      ".",
    ]);
    const diff = execute("working-diff", "git", [
      "diff",
      "--no-ext-diff",
      "HEAD",
      "--",
      ".",
    ]);
    const untracked = execute("working-untracked", "git", [
      "ls-files",
      "--others",
      "--exclude-standard",
      "-z",
    ])
      .split("\0")
      .filter(Boolean);
    const currentChanges =
      diff +
      untracked.map((filename) => fs.readFileSync(filename, "utf8")).join("\n");
    execute(
      "gitleaks-working",
      path.join(tools, "gitleaks" + suffix),
      [
        "stdin",
        "--redact",
        "--report-format",
        "sarif",
        "--report-path",
        path.join(directory, "gitleaks-working.sarif"),
        "--exit-code",
        "1",
      ],
      false,
      600000,
      currentChanges,
    );
    const dependencies = execute("go-imports", "go", [
      "list",
      "-mod=readonly",
      "-tags",
      "sqlite_fts5,sqlite_dbstat",
      "-deps",
      "./...",
    ]);
    assert(
      !/^golang.org\/x\/crypto\/openpgp(?:\/|$)/m.test(dependencies),
      "OpenPGP exception condition violated",
    );
    execute("govulncheck", path.join(tools, "govulncheck" + suffix), [
      "-tags",
      "sqlite_fts5,sqlite_dbstat",
      "-json",
      "./...",
    ]);
    execute("govulncheck-reachable", path.join(tools, "govulncheck" + suffix), [
      "-tags",
      "sqlite_fts5,sqlite_dbstat",
      "./...",
    ]);
    const rootModule = fs.readFileSync("go.mod", "utf8");
    fs.writeFileSync(
      path.join(directory, "go.mod"),
      rootModule.replace(/^replace .*$/gm, ""),
    );
    const locks = [
      path.join(directory, "go.mod"),
      path.resolve("app/pnpm-lock.yaml"),
    ];
    for (const architecture of ["amd64", "arm64"]) {
      const image = `litradar:go-test-${architecture}`;
      const identity = JSON.parse(
        execute(`inspect-${architecture}`, "docker", [
          "image",
          "inspect",
          "--format",
          "{{json .}}",
          image,
        ]),
      );
      assert.equal(identity.Architecture, architecture);
      const output = path.join(directory, architecture);
      fs.mkdirSync(output, { recursive: true });
      const container = execute(`create-${architecture}`, "docker", [
        "create",
        "--name",
        `litradar-inventory-${randomUUID()}`,
        "--network",
        "none",
        identity.Id,
      ]).trim();
      try {
        execute(`inventory-${architecture}`, "docker", [
          "cp",
          `${container}:/usr/share/doc/litradar/third-party/.`,
          output,
        ]);
        for (const binary of ["litradar", "obscura"]) {
          execute(`binary-${binary}-${architecture}`, "docker", [
            "cp",
            `${container}:/usr/local/bin/${binary}`,
            path.join(output, binary),
          ]);
        }
        for (const [source, name] of [
          ["/usr/lib/litradar/libsimple.so", "libsimple.so"],
          ["/usr/bin/pdftotext", "pdftotext"],
          ["/etc/ssl/certs/ca-certificates.crt", "ca-certificates.crt"],
        ]) {
          execute(`native-${name}-${architecture}`, "docker", [
            "cp",
            `${container}:${source}`,
            path.join(output, name),
          ]);
        }
      } finally {
        execute(`remove-${architecture}`, "docker", ["rm", container]);
      }
      for (const line of fs
        .readFileSync(path.join(output, "native.sha256"), "utf8")
        .trim()
        .split(/\r?\n/)) {
        const [sha256, filename] = line.split(/\s+/);
        assert.equal(
          digest(path.join(output, path.posix.basename(filename))),
          sha256,
        );
      }
      const goInventory = path.join(output, "go-inventory");
      for (const lock of ["go.mod", "go.sum"]) {
        assert.equal(
          fs
            .readFileSync(path.join(goInventory, lock), "utf8")
            .replace(/\r\n/g, "\n"),
          fs.readFileSync(lock, "utf8").replace(/\r\n/g, "\n"),
          `Image ${lock} differs from scanned source`,
        );
      }
      const goEnvironment = JSON.parse(
        fs.readFileSync(path.join(goInventory, "environment.json"), "utf8"),
      );
      assert.equal(goEnvironment.GOVERSION, "go1.27.1");
      assert.equal(goEnvironment.GOARCH, architecture);
      assert.equal(goEnvironment.GOOS, "linux");
      assert.equal(goEnvironment.CGO_ENABLED, "1");
      assert.equal(
        digest(path.join(output, "litradar")),
        fs
          .readFileSync(path.join(goInventory, "binary.sha256"), "utf8")
          .split(/\s+/)[0],
      );
      const compiledModules = fs.readFileSync(
        path.join(goInventory, "binary-modules.txt"),
        "utf8",
      );
      assert(
        compiledModules.includes("go1.27.1") &&
          compiledModules.includes("-tags=sqlite_fts5,sqlite_dbstat"),
        "Image compiler or SQLite tags differ",
      );
      for (const [inventory, roots] of [
        ["patched-sources.sha256", ["third_party"]],
        ["application-sources.sha256", ["cmd", "internal", "assets"]],
      ])
        verifyImageSources(
          fs.readFileSync(path.join(goInventory, inventory), "utf8"),
          roots,
        );
      execute(
        `govulncheck-binary-${architecture}`,
        path.join(tools, "govulncheck" + suffix),
        ["-mode=binary", path.join(output, "litradar")],
      );
      const helperRelease = JSON.parse(
        fs.readFileSync(path.join(output, "obscura-release.json"), "utf8"),
      );
      const archive = path.join(output, "image.tar");
      execute(`export-image-${architecture}`, "docker", [
        "save",
        "--output",
        archive,
        identity.Id,
      ]);
      const sbom = execute(
        `sbom-${architecture}`,
        "docker",
        [
          "run",
          "--rm",
          "--network",
          "none",
          "--read-only",
          "--user",
          "10001:10001",
          "--cap-drop",
          "ALL",
          "--security-opt",
          "no-new-privileges",
          "--env",
          "SYFT_CHECK_FOR_APP_UPDATE=false",
          "--env",
          "SYFT_CACHE_DIR=/tmp/syft",
          "--tmpfs",
          "/tmp:rw,nosuid,nodev,size=1g,mode=1777",
          "--mount",
          `type=bind,source=${output},target=/input,readonly`,
          syftImage,
          "scan",
          "docker-archive:/input/image.tar",
          "-o",
          "spdx-json",
        ],
        false,
        900000,
      );
      const document = JSON.parse(sbom);
      assert.equal(document.spdxVersion, "SPDX-2.3");
      assert(document.packages?.length > 0);
      document.packages.push({
        SPDXID: "SPDXRef-simple-tokenizer",
        name: "simple",
        versionInfo: "0.7.1",
        downloadLocation:
          "https://codeload.github.com/wangfenjin/simple/tar.gz/45db071ba8043ffe8a2e5dfe41f9d68fb477576c",
        filesAnalyzed: false,
        licenseConcluded: "MIT",
        licenseDeclared: "MIT OR GPL-3.0-or-later",
        copyrightText: "NOASSERTION",
        sourceInfo:
          "Source SHA256 d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61; Jieba disabled; binary SHA256 " +
          digest(path.join(output, "libsimple.so")),
      });
      document.relationships.push({
        spdxElementId: document.SPDXID,
        relationshipType: "DESCRIBES",
        relatedSpdxElement: "SPDXRef-simple-tokenizer",
      });

      document.packages.push({
        SPDXID: "SPDXRef-obscura-release",
        name: "Obscura official binary distribution",
        versionInfo: helperRelease.version,
        downloadLocation:
          helperRelease.release.replace("/tag/", "/download/") +
          "/" +
          helperRelease.assets[architecture].file,
        filesAnalyzed: false,
        licenseConcluded: "NOASSERTION",
        licenseDeclared: "NOASSERTION",
        copyrightText: "NOASSERTION",
        sourceInfo:
          "Upstream render+stealth binaries. Bundled dependency, native engine and license/source review waived by user (A13); no transitive completeness claim.",
      });
      document.relationships.push({
        spdxElementId: document.SPDXID,
        relationshipType: "DESCRIBES",
        relatedSpdxElement: "SPDXRef-obscura-release",
      });
      const sqliteSource = fs.readFileSync(
        "third_party/go-sqlite3/sqlite3-binding.c",
        "utf8",
      );
      document.packages.push({
        SPDXID: "SPDXRef-embedded-sqlite",
        name: "SQLite amalgamation",
        versionInfo: sqliteSource.match(
          /^#define SQLITE_VERSION\s+"([^"]+)"/m,
        )[1],
        downloadLocation:
          "https://www.sqlite.org/src/info/" +
          sqliteSource.match(
            /^#define SQLITE_SOURCE_ID\s+"[^\"]* ([a-f0-9]+)"/m,
          )[1],
        filesAnalyzed: false,
        licenseConcluded: "NOASSERTION",
        licenseDeclared: "NOASSERTION",
        copyrightText:
          "Public domain dedication in the retained SQLite amalgamation header",
        sourceInfo:
          "Embedded in the Go CGO binary; verified source SHA256 " +
          digest("third_party/go-sqlite3/sqlite3-binding.c"),
      });
      document.relationships.push({
        spdxElementId: document.SPDXID,
        relationshipType: "DESCRIBES",
        relatedSpdxElement: "SPDXRef-embedded-sqlite",
      });
      fs.writeFileSync(
        path.join(output, "image.spdx.json"),
        JSON.stringify(document, null, 2) + "\n",
      );
      report.images.push({
        architecture,
        imageId: identity.Id,
        helper: helperRelease,
        sbomSha256: digest(path.join(output, "image.spdx.json")),
        imageArchiveSha256: digest(archive),
      });
    }
    const osvFile = path.join(directory, "osv.json");
    execute(
      "osv",
      path.join(tools, "osv-scanner" + suffix),
      [
        "scan",
        "source",
        "--no-call-analysis=go",
        "--no-resolve",
        ...locks.map((filename) => `--lockfile=${filename}`),
        "--format=json",
        `--output-file=${osvFile}`,
      ],
      true,
    );
    const osv = JSON.parse(fs.readFileSync(osvFile, "utf8"));
    assert(Array.isArray(osv.results), "Incomplete OSV output");
    for (const result of osv.results)
      for (const entry of result.packages ?? [])
        for (const vulnerability of entry.vulnerabilities ?? [])
          accept(
            vulnerability.id,
            entry.package.name,
            entry.package.version,
            entry.package.ecosystem,
          );
    report.status =
      "Passed applicable checks with approved exceptions and Obscura waiver";
    return report;
  } catch (error) {
    report.status = "Failed";
    report.error = error.message;
    throw error;
  } finally {
    report.finished = new Date().toISOString();
    fs.writeFileSync(
      path.join(directory, "result.json"),
      JSON.stringify(report, null, 2) + "\n",
    );
  }
}
