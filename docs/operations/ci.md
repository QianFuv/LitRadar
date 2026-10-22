# CI execution and timing

CI runs all quality gates on main pushes and pull requests. Version increases and
explicit version retries additionally run packaging, platform smoke tests and
publication. Scheduled coverage remains informational. No path filter or cached
test result substitutes for a required gate.

## Quality jobs

- Release rules classify the push before the backend/frontend workflows start.
- Backend jobs run independent `regular` and `race` lanes on Linux and Windows.
  The regular lane owns formatting, module/dependency verification, explicit vet,
  full root tests and replacement-module regular tests. The race lane runs the
  full root and replacement-module race suites. Both initialize and verify native
  tokenizer inputs, compiler identity and diagnostics. Integration/SQLite/process
  tests remain in the full Go suites; neither lane uses `-short`.
- The two existing `Backend / Go checks (<os>)` names remain aggregate gates. Each
  requires the entire four-job matrix to succeed; failure, cancellation or skip
  cannot become a passing release prerequisite.
- Frontend contract generation, lint, formatting, types, jsdom, browser components,
  fixture flows and production full-stack flows all remain required. Their serial
  job is much shorter than the measured backend critical path.
- Only ordinary quality runs share a PR/ref concurrency group. Release-bearing
  runs use a separate group containing the run ID, so a later ordinary push cannot
  cancel a version's prerequisites. Publication keeps its existing version/latest
  serialization. There is no outer branch-wide cancellation of the release.

`node scripts/check-go.mjs` still runs all checks locally. Pass `regular` or `race`
to reproduce one CI lane. Do not run both lanes concurrently in the same working
directory: they share generated native files and diagnostic paths. Hosted matrix
jobs have separate checkouts.

## Cache and artifact ownership

Backend module caches explicitly include the root and both local replacement
modules' manifests. Compilation caches are separated by OS, architecture, exact
Go toolchain, lane, compiler and native build configuration; the final key includes
the source commit. Compatible prefix restores accelerate a new commit, which then
saves its own completed cache instead of freezing the first build under an
unchanged dependency key. Go still validates source inputs internally, and the
existing native archive/header fingerprint remains in `CGO_CFLAGS`.

Tests retain `-count=1`. Only root regular tests disable implicit vet, after the
same-config explicit `go vet` succeeds; race-selected files and dependency module
graphs retain their own vet behavior. Native outputs are rebuilt and checked,
rather than trusted because a cache path exists.

Next.js and pnpm caches remain distinct from test/build artifacts. Publication
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
correctness, not hosted performance. This optimization is validated locally only;
no new cloud CI run or release is authorized by this work. Record real post-change
job/step data after a separately authorized push before claiming a speedup.

## Official references and applicability

Release builds now reuse the full-stack job's source-bound export via its immutable
same-run artifact ID. Windows no longer installs pnpm/dependencies or builds Next.js;
the old web-build step took 12 seconds in the latest baseline and 21 seconds in the
comparison run. Linux overrides the Docker `frontend-build` stage using a named
context. Its frontend portion was not separately timed inside the 251-second Docker
step, so no numeric Linux saving is claimed. Artifact upload/download and hashing
add overhead; net savings remain unmeasured. CSP, full-stack and platform smoke
checks are retained. Hidden files and compressed representations travel intact.

- [GitHub artifact transfer](https://docs.github.com/en/actions/tutorials/store-and-share-data)
  supports cross-job reuse; automatic digest mismatch is a warning, so release
  consumers independently fail on source/file-inventory mismatch.
- [Docker named contexts](https://docs.docker.com/reference/cli/docker/buildx/build/#build-context)
  can override a same-named build stage. The
  [pinned build-push action](https://github.com/docker/build-push-action/blob/c3c9e263c25d99ce0380d002d59b67737d91b0dc/README.md)
  exposes this through `build-contexts`. Standalone source builds remain unchanged.

- [GoReleaser build workflow](https://github.com/goreleaser/goreleaser/blob/4762c79733dde55c7f63b3cdb4ef08d249df562d/.github/workflows/build.yml)
  separates test-cache identity from faster lint/license jobs and cancels obsolete
  builds; its [release workflow](https://github.com/goreleaser/goreleaser/blob/4762c79733dde55c7f63b3cdb4ef08d249df562d/.github/workflows/release.yml)
  is separate. LitRadar retains its combined entrypoint, with release-aware groups.
- [Caddy CI](https://github.com/caddyserver/caddy/blob/f355b8687d5582ea9fcc99f71598109d1e5b6ced/.github/workflows/ci.yml)
  uses independent OS jobs, `fail-fast: false`, and race testing. Splitting regular
  and race into separate lanes is LitRadar's measured-bottleneck decision, not a
  claim that Caddy has the identical design.
- [GitHub concurrency](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
  describes running cancellation and pending-run replacement. Distinct release
  groups are needed; setting cancellation false in one shared branch group alone
  would not protect releases from later ordinary runs.
- [Pinned setup-go cache behavior](https://github.com/actions/setup-go/blob/b7ad1dad31e06c5925ef5d2fc7ad053ef454303e/src/cache-save.ts)
  skips saving on an exact hit. Explicit source-aware build keys allow refreshed
  compilation caches without mixing regular and race writers.
- [Go test and caching](https://pkg.go.dev/cmd/go#hdr-Test_packages) documents
  implicit vet; [Go build caching](https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching)
  explains cgo external-library invalidation limitations. Native fingerprints and
  uncached test execution remain necessary here.
