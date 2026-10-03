# LitRadar MCP compatibility patch

Upstream: modelcontextprotocol/go-sdk v1.8.0, commit `3f3b699b2b67e1ed033a63d6651671dab53c2d32`. The source copy and original MIT license are retained under `../go-sdk`. `upstream.json` identifies every original byte; `patched.json` identifies the bounded changed/new files and `compatibility.patch`.

The opt-in mode adapts the existing SDK HTTP transport, session timers and streams to the frozen rmcp 2.1.0 contract. It does not add a second JSON-RPC dispatcher, session manager or event store. The SDK's default mode remains covered by its upstream suite. The application authenticates each request, validates Host/Origin and carries trusted version/principal context through public SDK middleware.

The helper includes legacy JSON syntax and envelope classification because malformed-request HTTP diagnostics are observable. Independent `from_reader` observations from fixed serde_json 1.0.150/rmcp 2.1.0 and real baseline HTTP responses test those boundaries. This is a maintained compatibility component, not a claim that Go's standard JSON decoder exactly matches serde. Representative primitive proofs do not close every business tool or the final API matrix.

The sole approved behavioral exception (A4) rejects a concurrent duplicate request ID within one session with HTTP 400 / JSON-RPC -32600, preserving the original response and avoiding duplicate execution. Completed IDs can be reused; sessions are independent.

## Verification and updates

Run `node tests/migration/run.mjs --phase sdk-integrity` from the repository root. It verifies the hash-pinned upstream archive, every original and added file, unchanged upstream module manifests/license, patch hashes, and exact patch reconstruction. An unrelated edit or extra file fails this check. The downloaded archive is evidence input; its absence is an error, not a skipped check.

Run both regular and race MCP tests inside the SDK's original module and under the application's build list, with `GOWORK=off`, on Windows and Linux. The `primitives` phase executes these and records the distinct build lists. Application-boundary and independent raw-client tests remain required in addition to upstream tests.

For a security upgrade, inspect upstream changes, rebase only the approved patch files, regenerate identities deliberately, and rerun the same integrity, default-mode, compatibility and application-build-list gates. Upstream release activity does not establish that this patch can be removed. Do not update the copied upstream `go.mod` or `go.sum` to make a test pass.
