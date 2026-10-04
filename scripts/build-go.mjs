/** Build the Go application and fixture binaries used by the real-backend browser suite. */
import { execFileSync } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const OUTPUT = path.join(ROOT, "target", "go");
await mkdir(OUTPUT, { recursive: true });
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
      "-tags",
      "sqlite_fts5,sqlite_dbstat",
      "-o",
      filename,
      `./cmd/${command}`,
    ],
    {
      cwd: ROOT,
      env: {
        ...process.env,
        CGO_ENABLED: "1",
        GOWORK: "off",
        GOENV: "off",
        GOFLAGS: "",
        GOTOOLCHAIN: "go1.27.1",
      },
      stdio: "inherit",
      windowsHide: true,
      timeout: 300000,
    },
  );
}
