# CFP compatibility evidence

Run `node tests/migration/run.mjs --phase cfp` on the approved Windows/WSL host.
The runner rejects stale exporters, builders, original source inventories,
dependency artifacts and fixture inputs. It builds the locked original Rust
observers, checks native Windows/Linux ordinary and race tests, verifies modules
and runs scoped vet. Missing required test passes or skipped native handoffs do
not satisfy the runner.

The independently generated corpora contain:

- 61 original storage histories, 206 transitions and 178 domain decisions.
- 124 observations from unchanged original source tests with observational call
  wrappers; all original assertions still run during export.
- 43 original worker observations for helper envelopes, Unicode title matching,
  cached full-text traversal, saved-capture identity, partial publication,
  cancellation and discovery retention. Observer code is appended to copies of
  original modules in ignored output; production Rust is unchanged.
- The original 1,622-notice seed, canonical notice hashes and fixed-time states.

Windows additionally performs 12 Rust/Go database handoffs and six saved-capture
handoffs on the same physical files. Native helper fixtures run on both operating
systems and verify cancellation, deadline, bounded output, typed arguments,
private-network environment removal and descendant cleanup after leader exit.
Concurrent import, generation fencing and consistent reader snapshots use real
SQLite connections. Publication-success/evidence-write-failure is separately
verified: the typed `EvidenceError` preserves the durable publication result.

Run `node tests/migration/run.mjs --phase cfp-live` separately for the pinned Linux
helper image and bounded public source probes. The actual Go helper adapter runs
under nonroot, read-only, capability-free, no-new-privileges container settings.
The synthetic rendering fixture alone enables a private-network override; the
production adapter must remove it and fail closed. Real Poppler extracts original
Chinese PDF text. The fixture container has no external network.

Public probes read one discovery page per automatic adapter, with a 20-second
budget each. They report fetched, parsed or blocked states independently. A
challenge, access failure or unrecognized live layout is never counted as a
successful acquisition. Full registry coverage is fixture-based; live checks
must be refreshed before cutover. The final application image and multiarchitecture
runtime remain later packaging gates. No same-origin subresource isolation or
separate helper/key filesystem isolation is claimed.

Regenerate corpora only after inspecting an intentional original observer change:
run `build-storage.mjs`, `export-storage.mjs`, `build-source.mjs`,
`export-source.mjs`, `build-worker.mjs`, `export-worker.mjs`, in that order.
Generated build manifests, native binaries, disposable databases and complete
verification logs live under ignored `output/migration/execution/`.

The implementation retains 4 MiB per page/helper output, 24 MiB per source
capture, 12 detail pages, two discovery workers, four full-text workers and
bounded leases. Failed acquisition preserves existing notices. Partial full-text
publication preserves unresolved originals and does not advance last-success.
The packaged source registry retains snapshot-only entries explicitly.

Bundled initialization also recognizes the exact historical CRLF checkout of the
reviewed LF seed. It preserves the existing marker and content rather than
reimporting or rewriting them. Ordinary operator imports retain exact-byte
identity checks; arbitrary whitespace or content changes still fail.
