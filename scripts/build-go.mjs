/** Build the Go application and fixture binaries used by the real-backend browser suite. */
import { execFileSync } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { stageWebAssets } from "./stage-web-assets.mjs";
import {
  buildSimple,
  simpleBuildEnvironment,
} from "./build-simple-tokenizer.mjs";

const ROOT = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const OUTPUT = path.join(ROOT, "target", "go");
await mkdir(OUTPUT, { recursive: true });
await stageWebAssets(ROOT);
await buildSimple(ROOT);
for (const command of ["litradar", "litradar-fixture"]) {
  const filename = path.join(
    OUTPUT,
    command + (process.platform === "win32" ? ".exe" : ""),
  );
  execFileSync(
    "go",
    [
      "build",
      "-mod=readonly",
      "-trimpath",
      "-tags",
      command === "litradar"
        ? "sqlite_fts5,sqlite_dbstat,litradar_web"
        : "sqlite_fts5,sqlite_dbstat",
      "-o",
      filename,
      `./cmd/${command}`,
    ],
    {
      cwd: ROOT,
      env: {
        ...simpleBuildEnvironment(ROOT),
        CGO_ENABLED: "1",
        GOWORK: "off",
        GOENV: "off",
        GOFLAGS: "",
        GOTOOLCHAIN: "go1.27.2",
      },
      stdio: "inherit",
      windowsHide: true,
      timeout: 300000,
    },
  );
}
