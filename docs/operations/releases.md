# Versioned releases

The root `VERSION` file is the release source of truth. It contains a stable
`MAJOR.MINOR.PATCH` version without a `v` prefix. The frontend's private
`apps/client/frontend/package.json` version does not trigger releases.

## Publish a version

1. Change `VERSION`, for example from `0.1.0` to `0.1.1`.
2. Commit that change together with the intended release contents and push to `main`.
3. CI compares the versions before and after the entire push, runs the backend, frontend and
   client quality checks, and builds only when the version increases.

Ordinary pushes run quality checks without building release images or archives.
Test suites may still compile binaries and the frontend for verification. The
initial `VERSION=0.1.0` establishes the existing version without publishing it.
Version downgrades and malformed versions fail CI. Tags pushed separately do not
trigger releases. No manual tag or release creation is needed.

After backend, frontend and client quality checks pass, release jobs build the server and clients from the exact source commit. Publication requires all mandatory jobs and validates transferred checksums, source SHA, server smoke evidence and client image identity. The public assets are:

- `litradar-server_<version>_linux_amd64.tar.gz`
- `litradar-server_<version>_windows_amd64.zip`
- `litradar-client-web_<version>_linux_amd64` (Wails Server Build binary)
- `litradar-client_<version>_windows_amd64_setup.exe` (unsigned installer)
- `litradar-client_<version>_android_arm64.apk` (release-signed, when configured)
- `SHA256SUMS` covering every published binary/package

Server OCI images remain `ghcr.io/qianfuv/litradar:v<version>`; Web client images use `ghcr.io/qianfuv/litradar-client-web:v<version>`. Both are linux/amd64 and both have serialized latest promotion. Server packagers continue to reject unsupported targets before extraction. Native clients are Windows x64 and Android arm64-v8a only; there are no iOS, macOS or Linux desktop jobs. [Wails officially marks mobile support experimental](https://wails.io/blog/wails-v3-released).

Release notes list every commit since the highest older published stable version
reachable from the release commit. Conventional Commit prefixes group entries by
type, with unrecognized subjects and merge commits retained under Other commits.
Each entry links to its full commit, and the notes include a full comparison link.
The first stable release lists the entire history. Draft retries regenerate the
notes; published release notes remain unchanged.

## Actions entrypoints

Server packaging is independent of UI builds. Client release jobs install pinned Task v3.54.0 and the Wails CLI matching `apps/client/go.mod`, install frontend dependencies from the root pnpm lockfile, and invoke these exact contracts in `apps/client`:

| Target | Command | Task output |
| --- | --- | --- |
| Web | `task build:server` | `bin/litradar-client-web` (linux/amd64, CGO disabled) |
| Windows | `task package:windows` | `bin/litradar-client-windows-amd64-setup.exe` |
| Android | `task package:android` | `bin/litradar-client-android-arm64.apk` |

Windows runs on a Windows x64 runner with NSIS; the installer is unsigned. Android uses an Ubuntu runner, Java 21 and sdkmanager with Android 35/build-tools 35.0.0 and NDK r28c (`28.2.13676358`). Quality jobs exercise all three Task targets; the Web host check belongs only to the client workflow. Android quality builds may use development signing.

### Android release signing

Configure all four repository secrets:

- `ANDROID_SIGNING_KEYSTORE`: base64-encoded release keystore. CI decodes it to a temporary file and passes its path as the `ANDROID_SIGNING_KEYSTORE` environment variable.
- `ANDROID_SIGNING_KEYSTORE_PASSWORD`
- `ANDROID_SIGNING_KEY_ALIAS`
- `ANDROID_SIGNING_KEY_PASSWORD`

The remaining three secrets map directly to environment variables with the same names. The Task must consume them for release signing. CI verifies the APK with apksigner and compares its signing certificate against the configured keystore using keytool, inventories only the output/evidence directory, and removes the decoded keystore in an always-running cleanup step. No signing file is uploaded.

If any secret is absent, the Android release lane records `android: omitted`, writes an explicit job-summary warning, and uploads omission evidence. Publication continues for the server, Web and Windows artifacts. It never substitutes a debug-signed release APK. The release notes explicitly state when Android was omitted. Complete signing configuration with an invalid key or password fails the release lane.

Client manifests bind the exact source, platforms and file hashes, including the image tar. The publish job verifies them before loading the accepted image; the loaded image must match its smoke-tested image ID, version and revision. Missing or changed required client artifacts fail publication.

- CI runs on main pushes and pull requests. It checks release rules, Linux/Windows
  Go code, the frontend and all client targets, then calls Release when VERSION increases.
- CI also accepts a manual version retry on main. It runs the same checks and
  complete release flow, with no separate supplement or promotion entrypoints.
- Release and the reusable check workflows are internal workflows only.
- Test Diagnostics runs weekly or manually for informational coverage artifacts.

Images are published as `ghcr.io/qianfuv/litradar:v<version>` and `:latest`.
Prefer a version tag for reproducible deployments. The Git tag
`v<version>` points to the exact CI source commit. A serialized promotion step
selects the highest published stable version for both latest pointers.

To retry the current version, select **Run workflow** on CI or run:

```sh
gh workflow run ci.yaml --ref main -f version=0.2.2
```

The requested version must match VERSION at the selected main commit. All
quality suites and all required build and smoke gates run. Existing tags and
drafts must belong to that same commit; published assets are never rebuilt or
replaced. A same-commit retry of a published release only updates latest pointers
and verifies the release after the quality checks.

If a workflow repair is needed before any tag or draft exists, push the repair
without changing VERSION and retry on the repaired main commit. Otherwise retry
the original run on its original commit. Never move a published tag to repair a
release; bump the version when a new source commit is required.

Image and GitHub publication are separate services. A transient failure may leave
a versioned image published while the release is still a draft; retrying the
original workflow resumes publication and updates latest only after publication.

## Run a binary archive

The Linux package targets Ubuntu 26.04 on amd64. It contains the native Go
server binary (with the statically linked Simple tokenizer),
the catalog bundle, Obscura and its worker, and third-party notices. It is a
dynamically linked distribution, not a standalone static binary. Install the system dependencies first:

```sh
sudo apt-get update
sudo apt-get install ca-certificates libgcc-s1 libstdc++6 poppler-utils poppler-data openssl
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf litradar-server_0.2.2_linux_amd64.tar.gz
cd litradar-server_0.2.2_linux_amd64
mkdir -p secrets
test -e secrets/litradar.key || (umask 077; openssl rand -out secrets/litradar.key 32)
./run.sh --version
./run.sh serve --secret-key-file secrets/litradar.key
```

`run.sh` selects the archive directory as the working directory and the bundled
Obscura helper. `pdftotext` is provided by the installed Poppler packages. To run
`litradar` directly, set `LITRADAR_OBSCURA_PATH` to the bundled Obscura and run
from the archive directory, or specify `--project-root` pointing to the writable
data root. The server serves API, docs and health endpoints; `/` returns 404. Deploy the Web client separately behind the gateway. Native metadata discovery uses the system bundle first and
then `assets/meta/` beside the executable; it preserves customized catalogs.

Keep deployment keys and existing `data/` across upgrades. Stop the old service
and back up its data before replacing executable/assets; never overwrite the
data directory or regenerate an existing deployment key. The same admin, index,
CFP and scheduling commands remain available through `run.sh`.

## Run on Windows x64

Extract the ZIP into a writable directory. Windows 10/11 or Windows Server with
PowerShell 5.1 is required; Go, Node.js, Docker and development tools are not
required. The distribution contains Obscura render/stealth, Poppler PDF extraction and
their native runtime libraries; the search tokenizer is linked into the executable. Normal Windows system
fonts are used for rendering.

In PowerShell, verify the ZIP against `SHA256SUMS`, then extract it:

```powershell
Get-FileHash .\litradar-server_0.2.2_windows_amd64.zip -Algorithm SHA256
Expand-Archive .\litradar-server_0.2.2_windows_amd64.zip -DestinationPath .
Set-Location .\litradar-server_0.2.2_windows_amd64
New-Item -ItemType Directory -Force secrets | Out-Null
if (-not (Test-Path secrets/litradar.key)) {
    $keyBytes = New-Object byte[] 32
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($keyBytes) } finally { $generator.Dispose() }
    [IO.File]::WriteAllBytes((Join-Path $PWD 'secrets/litradar.key'), $keyBytes)
}
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 --version
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 serve --secret-key-file secrets/litradar.key
```

The execution-policy option applies only to this launcher process. The launcher
selects its own directory and packaged helpers while preserving explicit helper
overrides. All CLI commands remain available. Keep `data/` and `secrets/` across
upgrades; do not unpack over a running service.

Server executables statically link Simple and contain no UI. Client binaries embed the shared Vite build. Release archives contain no external `web/`, `simple.dll` or `libsimple.so`. Windows packaging statically links the GCC/C++ runtime; Linux retains system C/C++ runtime dependencies. Obscura, Poppler, CA certificates and metadata catalogs retain their documented separate deployment requirements. Go inventory includes the static archive/source/compiler/header identity. Smoke tests relocate the executable into a fresh root and verify authenticated v9 migration/search without tokenizer libraries.

## Run the Web client binary

On Linux amd64, download `litradar-client-web_<version>_linux_amd64`, verify it against SHA256SUMS and mark it executable with `chmod +x`. Start it with WAILS_SERVER_HOST/WAILS_SERVER_PORT on the private network behind the same Caddy routes used by Compose. The binary serves the Vite SPA; the Go API server still runs separately. Windows and Android apps ask for the server URL and use native device tokens; see the [client guide](../guides/clients.md).
