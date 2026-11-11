# CI execution and timing

CI runs all quality gates on main pushes and pull requests. Version increases and
explicit version retries additionally run packaging, platform smoke tests and
publication. Scheduled coverage remains informational. No path filter or cached
test result substitutes for a required gate.

## Shared workflow contract

All workflow YAML uses two-space structural indentation and the same stage vocabulary:
`detect-changes` -> `quality` -> `build-acceptance` -> `publish`. Existing workflow
names, job IDs and explicit or implicit job names are compatibility interfaces and
remain unchanged. Stage labels and summaries make the flow consistent without
renaming required checks. Detection classifies version changes across the entire
push; it does not filter paths or skip required quality checks. Normal pushes run
quality and its production frontend build; release packaging runs only for a version
increase or explicit same-version retry.

Every transferred artifact has source SHA, target platform and file SHA-256 evidence.
The common `ci-manifest.json` schema records `sourceSha`, `platforms`, `stage`,
`status`, check outcomes/durations, missing paths and artifact-relative file hashes.
Platform-independent exports say `independent`; combined publication evidence lists
both targets. Large image tar files are hashed as streams. Manifests stay inside
existing artifact roots and outside official release `assets/`, preserving download
layouts and the versioned public server and client assets. Client evidence also binds its image tar and optional signed Android package.

Successful build transfers require actual payloads before upload; a manifest alone
cannot satisfy the contract. Diagnostic reports may record missing files after an
earlier failure. Final summaries run after uploads and report the current job status,
so a failed upload cannot be presented as success. The same heading, source/platform
fields and check/status/duration table apply to all stages; existing frontend timing
and flaky-test policy, and informational coverage policy, remain visible.

Go caches explicitly name the root and go-sdk replacement module's `go.mod`/`go.sum`
inputs. Backend compilation caches remain partitioned by lane and, for the race
lane, compiler identity. pnpm names
the root lockfile. BuildKit
uses the `litradar-linux-amd64` cache scope and hashes Dockerfile, build contexts,
arguments and pinned bases through its own cache graph. Cached bytes never replace
tests, source-bound frontend verification or platform smoke acceptance.

## Quality jobs

- Release rules classify the push before the backend/frontend workflows start.
- Backend jobs run independent `regular` and `race` lanes on Linux and Windows.
  The regular lane owns formatting, module/dependency verification, explicit vet,
  full root tests and replacement-module regular tests. The race lane runs the
  full root and replacement-module race suites. The regular lane builds with
  `CGO_ENABLED=0` and needs no C compiler; the race lane uses `CGO_ENABLED=1`,
  because Go's race detector requires cgo, and records compiler identity. Both
  record diagnostics. Integration/SQLite/process tests remain in the full Go suites;
  neither lane uses `-short`.
- The regular lane also runs `golangci-lint run . ./apps/server/...` with the root
  `.golangci.yml` (which owns the enabled linters), checks the migration-derived
  sqlc schema snapshot with `go run ./apps/server/cmd/sqlc-schema -check`, and runs
  `sqlc diff -f apps/server/sqlc.yaml`. Each writes a log under `test-results/go/`.
  The race lane does not install these tools.
- The two existing `Backend / Go checks (<os>)` names remain aggregate gates. Each
  requires the entire four-job matrix to succeed; failure, cancellation or skip
  cannot become a passing release prerequisite.
- Frontend contract generation, `pnpm check` (Vite+ `vp check`: Oxfmt formatting,
  Oxlint and TypeScript 7 type checking), Knip, jsdom, browser components,
  fixture flows and production full-stack flows all remain required. Their serial
  job is much shorter than the measured backend critical path.
- Only ordinary quality runs share a PR/ref concurrency group. Release-bearing
  runs use a separate group containing the run ID, so a later ordinary push cannot
  cancel a version's prerequisites. Publication keeps its existing version/latest
  serialization. There is no outer branch-wide cancellation of the release.

`node apps/server/scripts/check-go.mjs` still runs all checks locally. Pass `regular` or `race`
to reproduce one CI lane. Do not run both lanes concurrently in the same working
directory: they share diagnostic paths. Hosted matrix
jobs have separate checkouts.

## Cache and artifact ownership

Backend module caches explicitly include the root and go-sdk local replacement
module manifests. Compilation caches are separated by OS, architecture, exact
Go toolchain, lane and race-lane compiler identity; the final key includes
the source commit. Compatible prefix restores accelerate a new commit, which then
saves its own completed cache instead of freezing the first build under an
unchanged dependency key. Go still validates source inputs internally.
The regular lane caches the golangci-lint and sqlc binaries under a key made of OS,
architecture, Go toolchain and both pinned tool versions
(`GOLANGCI_LINT_VERSION` and `SQLC_VERSION` in `backend.yaml`).

Tests retain `-count=1`. Only root regular tests disable implicit vet, after the
same-config explicit `go vet` succeeds; race-selected files and dependency module
graphs retain their own vet behavior.

Vite and pnpm caches remain distinct from test/build artifacts. Publication
continues transferring the exact smoke-tested Linux image and both platform
archives to its publication job; it never rebuilds those bytes there.

## Measured baseline (2026-10-23 analysis)

Source: [run 39633240092](https://github.com/QianFuv/LitRadar/actions/runs/39633240092),
commit `d4a0478660a843e6f3e6eddc6f97f9490829c58a`, Go 1.27.3. Values come from
GitHub's job/step timestamps and each backend artifact's `results.json`.

| Successful job              | Job time | Dominant step                                       |
| --------------------------- | -------: | --------------------------------------------------- |
| Windows backend (attempt 2) |  34m 31s | Combined checks: 33m 28s                            |
| Linux backend               |   9m 55s | Combined checks: 9m 35s                             |
| Frontend                    |   5m 43s | Fixture flows: 1m 43s; static checks: 1m 13s        |
| Linux release               |   5m 39s | Docker build: 4m 11s                                |
| Windows release             |   3m 13s | Package/smoke: 1m 29s; web build: 12s               |
| Publication                 |   1m 26s | Download Linux bytes: 17s; release preparation: 15s |

The run was created at 16:16:11 UTC, the retry started at 16:55:06, and completion
was 17:38:07. Its 81m 56s elapsed time includes a failed Windows attempt and the
retry gap, not 81 minutes of successful job work. The prior successful release
[run 39111097654](https://github.com/QianFuv/LitRadar/actions/runs/39111097654)
also had a 35m 10s Windows backend job (33m 26s checks), confirming the bottleneck.

Within the latest successful Windows artifact, root regular tests took 589.28s,
root race 658.36s, explicit vet 55.27s, and the eight replacement-module commands
687.81s combined. The module-own and application-root dependency graphs differ;
these tests are retained, not discarded as duplicates.

## Expected versus measured savings

Adding the recorded command durations into the new lanes gives approximately
15m 28s regular and 17m 46s race on Windows, versus 33m 28s for the old combined
step. With identical command costs and simultaneous runner availability, that
models about 15m 42s less critical-path time before setup/cache/queue differences.
It is a scheduling estimate, not a hosted measurement. Extra workers can increase
runner minutes, and cold caches or queue delays can reduce the benefit.

Cache refresh and avoiding repeated regular-root vet may save compilation work,
but their separate impact has not been measured. Local contract tests measure
correctness, not hosted performance. The initial optimization was validated locally;
record actual post-change job/step data from an authorized hosted run before claiming
a speedup. Version releases and deployments require their own authorization.

## Client quality and releases

`Client` adds Web linux/amd64, Windows x64 and Android arm64 quality lanes. Task is pinned to v3.54.0; Wails CLI follows the version in the shared root Go module. The client workflow owns the Web host check; frontend.yaml owns Vite contracts, static checks and browser flows.

Server release builds never download frontend exports. Web release builds embed the same single Vite frontend and smoke `/` and `/login`; Windows invokes `task package:windows`; Android invokes `task package:android` with Java 21, SDK 35 and NDK r28c. Release APKs require all four signing secrets. Missing signing configuration yields explicit omission evidence and job-summary warnings while mandatory packages still publish. The Windows portable client is unsigned. See [release contracts](releases.md).

Publication verifies source and SHA-256 inventories before loading client image bytes, checks the loaded image against smoke evidence, and runs the isolated three-service gateway smoke. Existing server archive/image evidence, exact-commit retries and serialized highest-version latest promotion remain required. Public assets include server archives, Web binary, Windows portable zip, optional signed Android APK and one SHA256SUMS. Both OCI images are linux/amd64. Hosted run timings must be measured before making performance claims.

- [GitHub concurrency](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
  describes running cancellation and pending-run replacement. Distinct release
  groups are needed; setting cancellation false in one shared branch group alone
  would not protect releases from later ordinary runs.
- [Pinned setup-go cache behavior](https://github.com/actions/setup-go/blob/b7ad1dad31e06c5925ef5d2fc7ad053ef454303e/src/cache-save.ts)
  skips saving on an exact hit. Explicit source-aware build keys allow refreshed
  compilation caches without mixing regular and race writers.
- [Go test and caching](https://pkg.go.dev/cmd/go#hdr-Test_packages) documents
  implicit vet; [Go build caching](https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching)
  explains cgo external-library invalidation limitations. Uncached test execution
  remains necessary here.
