# Source migration proof

Run `node tests/migration/run.mjs --phase sources` from the repository root with
the approved Go 1.27.1 Windows/MinGW and WSL Ubuntu toolchains available. The
runner writes `output/migration/execution/sources-result.json`, including source
identities, exact commands, logs and current results. It fails on missing or
stale expected observations, changed original Rust inputs, formatting failures,
failed native/race checks, or skipped Windows workset interoperability.

The frozen observations come from the original locked Rust dependency graph.
`build-oracles.mjs` builds nine isolated observers without editing production
Rust. Private helper copies, appended observers and the two fixed-clock seams
are identified by source hashes in each corpus. Expected results are never
generated from the Go implementation. Regeneration is a separate authoring
operation: build the oracles, then run the explicit `export-*.mjs` programs.
The normal proof runner rebuilds current oracle binaries and verifies their
inputs against the frozen corpus; it does not replace expected results.

Coverage includes numeric/JSON compatibility, provider contracts, explicit
proxy and DNS behavior, retries and quotas, HTTP wire behavior, CNKI parsing and
captcha, ZJLib login/session/PDF handling, Crossref owned SQLite worksets,
checkpoint recovery, canonical conversion, complete indexing workflows and
request-time access adapters. Large workset tests collect and emit 100,001
records in both dense and distributed layouts. Real local HTTP, TLS and proxy
servers exercise the transport path without contacting production accounts.

Windows tests exchange live workset files with the unchanged Rust implementation
in both directions. Linux runs the same frozen observations and native/race
tests, with the Windows executable handoff explicitly skipped. Native SQLite
uses `sqlite_fts5`; full storage/application proofs additionally need the
previously established `sqlite_dbstat` build tag.

Known boundaries remain explicit: malformed HTML ordering requires a small
source-specific adapter, and finite URL/HTML observations do not prove universal
parser equivalence. Source access implementations do not perform API routing or
persist refreshed access sessions. Those runtime and HTTP integration checks
belong to the later migration tasks.
