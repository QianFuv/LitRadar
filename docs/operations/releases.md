# Versioned releases

The root `VERSION` file is the release source of truth. It contains a stable
`MAJOR.MINOR.PATCH` version without a `v` prefix. The frontend's private
`app/package.json` version does not trigger releases.

## Publish a version

1. Change `VERSION`, for example from `0.1.0` to `0.1.1`.
2. Commit that change together with the intended release contents and push to `main`.
3. CI compares the versions before and after the entire push, runs the backend and
   frontend quality checks, and builds only when the version increases.

Ordinary pushes run quality checks without building release images or archives.
Test suites may still compile binaries and the frontend for verification. The
initial `VERSION=0.1.0` establishes the existing version without publishing it.
Version downgrades and malformed versions fail CI. Tags pushed separately do not
trigger releases. No manual tag or release creation is needed.

After the backend and frontend checks pass, Windows x64 and Linux amd64 build
and smoke jobs run in parallel. Publication waits for both, verifies the transferred
image against its smoke-tested identity, and prepares a draft with all tested
binaries. It publishes the Linux image, publishes the draft, updates the latest
pointers, and verifies the public release. Required skipped jobs fail the final
completion check. The release contains:

- `litradar_<version>_linux_amd64.tar.gz`
- `litradar_<version>_windows_amd64.zip`
- `SHA256SUMS`

`SHA256SUMS` covers both archives. Releases do not publish a separate Windows
checksum file. ARM64 builds and partial platform supplementation are not supported.

Release notes list every commit since the highest older published stable version
reachable from the release commit. Conventional Commit prefixes group entries by
type, with unrecognized subjects and merge commits retained under Other commits.
Each entry links to its full commit, and the notes include a full comparison link.
The first stable release lists the entire history. Draft retries regenerate the
notes; published release notes remain unchanged.

## Actions entrypoints

- CI runs on main pushes and pull requests. It checks release rules, Linux/Windows
  Go code and the frontend, then calls Release when VERSION increases.
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
gh workflow run ci.yaml --ref main -f version=0.2.1
```

The requested version must match VERSION at the selected main commit. Both
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

The Linux package targets Ubuntu 26.04 on amd64. They contain the native Go
binary with embedded web assets and CSP, catalog bundle, SQLite tokenizer, Obscura and its worker, and
third-party notices. They are dynamically linked distributions, not standalone
static binaries. Install the system dependencies first:

```sh
sudo apt-get update
sudo apt-get install ca-certificates libgcc-s1 libstdc++6 poppler-utils poppler-data openssl
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf litradar_0.1.1_linux_amd64.tar.gz
cd litradar_0.1.1_linux_amd64
mkdir -p secrets
test -e secrets/litradar.key || (umask 077; openssl rand -out secrets/litradar.key 32)
./run.sh --version
./run.sh serve --secret-key-file secrets/litradar.key
```

`run.sh` selects the archive directory as the working directory and the bundled
Obscura helper. `pdftotext` is provided by the installed Poppler packages. To run
`litradar` directly, set `LITRADAR_OBSCURA_PATH` to the bundled Obscura and run
from the archive directory, or specify `--project-root` pointing to the writable
data root. No external `web/` is required or consulted. Embedded assets use content
ETags, omit `Last-Modified`, and ignore date-based HTTP conditions. Native metadata discovery uses the system bundle first and
then `assets/meta/` beside the executable; it preserves customized catalogs.

Keep deployment keys and existing `data/` across upgrades. Stop the old service
and back up its data before replacing executable/assets; never overwrite the
data directory or regenerate an existing deployment key. The same admin, index,
CFP and scheduling commands remain available through `run.sh`.

## Run on Windows x64

Extract the ZIP into a writable directory. Windows 10/11 or Windows Server with
PowerShell 5.1 is required; Go, Node.js, Docker and development tools are not
required. The distribution contains Obscura render/stealth, the search tokenizer,
Poppler PDF extraction and their native runtime libraries. Normal Windows system
fonts are used for rendering.

In PowerShell, verify the ZIP against `SHA256SUMS`, then extract it:

```powershell
Get-FileHash .\litradar_0.2.0_windows_amd64.zip -Algorithm SHA256
Expand-Archive .\litradar_0.2.0_windows_amd64.zip -DestinationPath .
Set-Location .\litradar_0.2.0_windows_amd64
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
