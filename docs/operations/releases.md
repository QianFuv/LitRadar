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

After both architectures pass container smoke tests, CI prepares a draft GitHub
Release with the exact image binaries, publishes the multi-platform image, then
publishes the draft. The release contains:

- `litradar_<version>_linux_amd64.tar.gz`
- `litradar_<version>_linux_arm64.tar.gz`
- `SHA256SUMS`

Images are published as `ghcr.io/qianfuv/litradar:v<version>` and `:latest`, with
architecture-specific version tags. Prefer the version tag for reproducible
deployments. The Git tag `v<version>` points to the exact push-tip commit, even
when the version change is an earlier commit within the same push.

Version builds can run independently. A short serialized step selects the highest
published stable version for both `latest` pointers, so retrying an older release
cannot roll them back. Ordinary pushes cannot cancel a release's quality checks.

If publication fails, rerun that original workflow on the same commit. The same
draft may be completed; an already published release is skipped. A version tag
owned by a different commit is rejected. A later ordinary commit does not retry
a release implicitly. Image and GitHub publication are separate services, so a
failure between them can leave the image published while the release remains a
draft. Never move a published version tag to repair a release; bump the version.

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

Windows remains a supported development/test target; this release pipeline
publishes the two Linux targets already exercised by the container release gate.
