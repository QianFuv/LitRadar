# Go optimization round 1

This round addresses delivery failure handling, query cancellation, MCP resource
budgets, candidate selection, rate-limit eviction, shared package ownership and
route matching. It does not change the database schema or introduce dependencies.
Security scanning remains removed. This note distinguishes correctness changes,
local measurements, package organization and deferred work.

## Correctness and resource ownership

- Failed article selection returns its error and preserves retry checkpoints;
  recovery attributes unknown deliveries only to the same run and subscriber.
- REST index and MCP reads propagate cancellation, and obsolete frontend article
  requests abort. Admitted writes retain ownership until completion. Heartbeat
  execution no longer waits for user storage admission.
- MCP first reads enforce a 4 MiB raw-body limit (413 above the limit). The bounded
  normalized representation retains protocol/error compatibility. Authentication,
  origin and header precedence remain covered by boundary tests.
- HTTP header/idle budgets are 5/60 seconds. Each SSE event has a renewed 10-second
  write budget, while consuming streams continue across multiple keepalives.
  Network drain starts its 30-second deadline before session closure, force-closes
  stalled sockets and joins owned handlers. It is not a 30-second bound on all
  admitted database work or native SQLite lock latency.

The executor is owned by `internal/platform/executor`. Shared JSON validation and
encoding live in `internal/compat/jsonvalue`; 36 direct caller files migrated
without changing byte/error policy. The original source-specific numeric encoder
remains separate. These package moves are maintainability changes, not speedups.

## Reproducible performance baseline

Run from the repository root with the configured Go 1.27.1 CGO toolchain and
existing native Simple library:

```text
go test -run '^$' -bench 'Benchmark(Candidates|ArticleLookup|RateLimit|Route)' -benchmem -benchtime=1s -count=5 -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./internal/storage/query ./internal/api
```

The deterministic database contains 60,000 synthetic articles, eight journals and
32 issues in the real v9 schema, including the Simple FTS projection. Dates contain
ties and NULLs; every fourth article is in press. No ANALYZE is applied. Setup and
input construction are excluded from timing. The 32,767 and 65,536-ID probes are
reported separately from successful timings because the original query exceeds
SQLite's parameter limit.

Identifier lookup benchmarks exercise the complete ListArticles path, with and
without total counts. OpenSimple forces a physical connection and SELECT before
closing. WarmSelect is only a diagnostic comparison, not a production-pooling
implementation or an additive decomposition of total request time.

Limiter benchmarks churn both full keyed maps using precomputed clients and a
synthetic advancing clock; every request must be admitted. Route benchmarks use
the production route inventory and a fresh shallow request copy so path capture
allocations remain visible. No returned handler executes during route timing.

Record the five raw samples, source/fixture/native hashes, CPU and tool versions
under ignored `output/go-optimization-round-1/`. Compare equivalent fixtures on
the same machine. End-to-end throughput and production speedups are not inferred
from microbenchmarks.

## Current implementation status

Baseline captured on Windows/amd64, Go 1.27.1, Intel Core i9-12900H, 20 logical
processors. Each baseline median below uses five one-second samples of the
original implementation. Raw baseline and diagnostic logs are saved under
`output/go-optimization-round-1/`.

| Case | Median ns/op | Median B/op | Median allocs/op |
|---|---:|---:|---:|
| BenchmarkCandidates/Ids120 | 4.57602e+06 | 114031 | 3219 |
| BenchmarkCandidates/Ids500 | 5.36455e+06 | 453971 | 13430 |
| BenchmarkCandidates/Ids4096 | 1.51547e+07 | 4.38536e+06 | 112596 |
| BenchmarkCandidates/Ids32766 | 1.02324e+08 | 3.64474e+07 | 903137 |
| BenchmarkRateLimit/Churn64_64 | 1604 | 70 | 3 |
| BenchmarkRateLimit/Churn8192_4096 | 129766 | 71 | 3 |
| BenchmarkRoute/First | 12152 | 14064 | 275 |
| BenchmarkRoute/Middle | 12077 | 14036 | 268 |
| BenchmarkRoute/Last | 12051 | 14036 | 268 |
| BenchmarkRoute/Missing | 9411 | 10795 | 187 |
| BenchmarkArticleLookup/doi/hit=true/total=true | 1.33515e+08 | 19600 | 377 |
| BenchmarkArticleLookup/doi/hit=true/total=false | 1.34814e+08 | 18998 | 362 |
| BenchmarkArticleLookup/doi/hit=false/total=true | 1.36549e+08 | 22518 | 217 |
| BenchmarkArticleLookup/doi/hit=false/total=false | 1.35034e+08 | 13638 | 201 |
| BenchmarkArticleLookup/pmid/hit=true/total=true | 1.37272e+08 | 19580 | 378 |
| BenchmarkArticleLookup/pmid/hit=true/total=false | 1.36812e+08 | 36490 | 363 |
| BenchmarkArticleLookup/pmid/hit=false/total=true | 1.42817e+08 | 14221 | 217 |
| BenchmarkArticleLookup/pmid/hit=false/total=false | 1.37069e+08 | 13690 | 202 |
| BenchmarkArticleLookup/OpenSimple | 8.12913e+06 | 10855 | 159 |
| BenchmarkArticleLookup/WarmSelect | 2439 | 512 | 13 |

Untimed probes at 32,767 and 65,536 IDs both returned `too many SQL variables`.
Both DOI and PMID listing-count plans scan the listing; pagination scans its date
index. The canonical articles table has covering identifier indexes, but this does
not prove a compatible query rewrite for divergent projections. Physical-open
and warm-select timings are diagnostics with different lifetimes, not promised
production savings.

## Candidate membership correction

All three selectors now accept 32,767 and 65,536 distinct IDs. Above 500 IDs,
bounded inserts populate a connection-local TEMP table and one SELECT retains
SQLite's original date/ID ordering, journal join and exact in-press predicate.
There is no result truncation, persistent schema change or connection pool.
Executing-query cancellation and insert failures clean membership before the
same connection is reused; caller identifiers remain unchanged.

The initial rerun showed substantial host timing variation, including unchanged
lookup code. After retaining the original direct SQL construction for small sets,
a controlled comparison ran both implementations on the same fixture in one
process (five samples each):

| IDs | Old median ns/op | New median ns/op | Old/new allocs/op |
|---|---:|---:|---:|
| 120 | 5151863 | 4956910 | 3219 / 3218 |
| 500 | 5833909 | 6242977 | 13430 / 13430 |

The small-set difference stays within the 10% regression threshold. Large-set
timings varied and do not demonstrate a speedup; this change removes parameter
overflow while keeping individual SQL statements bounded. Raw initial, controlled
and final small-case samples are retained in `t6-*.log` and `t6-summary.json`.

## Rate-limit eviction

Normal keyed access and eviction now use maps plus standard-library lists. Full
maps reuse their oldest node; clearUsername removes both representations. The
rare saturated sequence fallback retains the previous lexical tie behavior.
List/key memory is bounded by the same configured capacities.

Five-sample medians on the original churn fixtures:

| Case | Before ns/op | After ns/op | Before/after B/op | Before/after allocs/op |
|---|---:|---:|---:|---:|
| Churn64_64 | 1604 | 335.2 | 70 / 22 | 3 / 1 |
| Churn8192_4096 | 129766 | 399.5 | 71 / 23 | 3 / 1 |

These measure full-capacity limiter churn, not overall HTTP throughput. The
reference trace compares 18,000 mixed operations including saturation and clear;
race and concurrent accounting tests pass. Raw samples: `t7-benchmark.log`.

## Precompiled route matching

Route patterns, lexicographic specificity and exact-template Allow metadata are
compiled once at construction. Requests split their escaped path once, and only
the selected route writes captures. Percent-escape validation can still allocate
while checking candidates; this is not an allocation-free routing claim.

| Case | Before ns/op | After ns/op | Before/after B/op | Before/after allocs/op |
|---|---:|---:|---:|---:|
| First | 12152 | 4449 | 14064 / 64 | 275 / 1 |
| Middle | 12077 | 3449 | 14036 / 64 | 268 / 1 |
| Last | 12051 | 3238 | 14036 / 64 | 268 / 1 |
| Missing | 9411 | 2118 | 10795 / 64 | 187 / 2 |

Each value is the median of five samples on the original 86-route fixtures;
compilation is outside timing and request copies retain capture costs. These
measure matching only, without invoking the selected handler. Raw full-suite
samples are in `t9-benchmark.log` and `t9-summary.json`. Compatibility tests cover
static priority, declaration-order HEAD/duplicate methods, 405 Allow ordering,
encoded slashes, invalid bytes/escapes, repeated parameters and frontend fallback.

## Integrated acceptance

The final acceptance commands are `node scripts/check-go.mjs` and the existing
frontend generated-API, lint, format, type, unit, browser-component, fixture and
real-backend suites. Their actual outcomes and input identities are recorded in
the task evidence; the microbenchmarks do not replace those checks. Windows is
this round's required platform. No dual-architecture release result is implied.

## Deferred work

Persistent query pools need explicit ownership, TEMP cleanup, file-generation and
restore semantics. DOI/PMID lookup rewrites need a decision about canonical versus
listing divergence; new listing indexes also require a schema contract. These are
not silently enabled by diagnostic benchmark results. Broad frontend splitting,
regex/charset rewrites, delivery batching and scheduler/transport changes remain
separate follow-up candidates.
