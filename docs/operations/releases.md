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

After Windows archive smoke and both Linux architectures pass container and
archive smoke tests, Release prepares a draft with all tested binaries, publishes
the multi-platform Linux image, then publishes the draft. The release contains:

- `litradar_<version>_linux_amd64.tar.gz`
- `litradar_<version>_linux_arm64.tar.gz`
- `litradar_<version>_windows_amd64.zip`
- `SHA256SUMS`

## Actions entrypoints

| Workflow                                      | Trigger                          | Responsibility                                                                                               |
| --------------------------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| CI                                            | Push to main or pull request     | Release rule tests, Linux/Windows Go checks, frontend checks; calls Release only when main VERSION increases |
| Release                                       | Called by CI or manually on main | Build, smoke, publish; explicit recovery operations below                                                    |
| Reusable Go Checks / Reusable Frontend Checks | Internal calls only              | One implementation of each quality suite; no separate push triggers                                          |
| Test Diagnostics                              | Weekly or manual                 | Informational coverage artifacts                                                                             |

Normal CI never builds release archives or images for an unchanged version.
Manual release operations rerun both quality suites at the selected source commit.
The Windows supplement builds the original tag's product code using packaging
tools from the selected main commit; `build.json` records both identities.

Images are published as `ghcr.io/qianfuv/litradar:v<version>` and `:latest`, with
architecture-specific version tags. Prefer the version tag for reproducible
deployments. The Git tag `v<version>` points to the exact push-tip commit, even
when the version change is an earlier commit within the same push.

Version builds can run independently. A short serialized step selects the highest
published stable version for both `latest` pointers, so retrying an older release
cannot roll them back. Ordinary pushes cannot cancel a release's quality checks.

If a code or workflow repair is needed before a version has acquired a tag or
draft release, push the repair without changing `VERSION`, then explicitly retry
the current version on `main`:

```sh
gh workflow run release.yaml --ref main -f operation=release -f version=0.2.0
```

The requested version must match `VERSION` at that workflow's commit. Both
quality suites and all build and smoke gates run again. Existing tags and drafts
must belong to the same commit; this command cannot move release ownership to a
different commit. Ordinary pushes still do not build or publish unchanged versions.

For a transient publication failure, rerun that original workflow on the same commit. The same
draft may be completed; an already published release is skipped. A version tag
owned by a different commit is rejected. A later ordinary commit does not retry
a release implicitly. Image and GitHub publication are separate services, so a
failure between them can leave the image published while the release remains a
draft. Never move a published version tag to repair a release; bump the version.

If a deterministic publication-script defect occurs after a draft or versioned
image exists, repair and verify the script without changing the release identity.
Before recovery, verify the original run's quality, architecture smoke results,
archive checksums, draft target commit and image revisions. With authenticated
GitHub access, run only the corrected `release-github.mjs publish` command with
`RELEASE_VERSION` and `GITHUB_SHA` set to that original version and full commit.
Do not run `prepare`, re-upload assets, or dispatch the version from the repair
commit. After publication, dispatch the promotion-only recovery workflow:

```sh
gh workflow run release.yaml --ref main -f operation=promote
```

It uses the normal `release-latest` concurrency lock and GitHub token, selects the
highest public stable release, and never builds or replaces versioned assets.
Record the release source
commit and recovery-script commit separately. Preserve the failed CI run and
record manual publication/promotion evidence instead of reporting it as green.

To add the missing Windows package to an already public version:

```sh
gh workflow run release.yaml --ref main -f operation=windows -f version=0.2.0
```

This requires a public stable tag reachable from main and matching the source
VERSION. It adds only the Windows ZIP and `<zip-name>.sha256`. Existing Linux
archives, `SHA256SUMS`, tag and images remain unchanged. If either Windows asset
already exists, its size and digest must match; the workflow never overwrites it.
The separate checksum is intentional for supplemented releases. Future normal
releases list all three archives in their original `SHA256SUMS`.

If upload stops after adding only one Windows asset, use **Re-run failed jobs**
to reuse the already tested `windows-release` artifact. A full rebuild can change
ZIP timestamps or build bytes and is deliberately rejected when it conflicts with
an existing asset. Never delete the published file to work around this check.

## Run a binary archive

The Linux packages target Debian 13 on amd64 or arm64. They contain the native Go
binary, web assets, catalog bundle, SQLite tokenizer, Obscura and its worker, and
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
from the archive directory, or specify `--project-root` pointing to a directory
containing `web/`. Native metadata discovery uses the system bundle first and
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

In PowerShell, verify the ZIP against `SHA256SUMS` (or its `.zip.sha256` sidecar
for a supplemented release), then extract it:

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
