# LitRadar MCP compatibility patch

Upstream: modelcontextprotocol/go-sdk v1.8.0, commit `3f3b699b2b67e1ed033a63d6651671dab53c2d32`. The source copy and original MIT license are retained under `../go-sdk`. `upstream.json` identifies every original byte; `patched.json` identifies the bounded changed/new files and `compatibility.patch`.

The opt-in mode adapts the existing SDK HTTP transport, session timers and streams to the application protocol contract. It does not add a second JSON-RPC dispatcher, session manager or event store. The SDK's default mode remains covered by its upstream suite. The application authenticates each request, validates Host/Origin and carries trusted version/principal context through public SDK middleware.

The helper normalizes JSON syntax and classifies envelopes to preserve observable malformed-request HTTP diagnostics. Independent Go and raw-client tests cover these application boundaries alongside the upstream transport suite.

The application rejects a concurrent duplicate request ID within one session with HTTP 400 / JSON-RPC -32600, preserving the original response and avoiding duplicate execution. Completed IDs can be reused; sessions are independent.

## Verification and updates

Resource subscription tests wait for the protocol acknowledgement before emitting
updates. The upstream handler callback runs before the subscription is registered,
so observing that callback alone can lose updates under concurrent scheduling.
This test-only change preserves update, subscription identity and unsubscribe
assertions; production subscription behavior is unchanged.

Run `node scripts/verify-go-dependency.mjs go-sdk` from the repository root. It verifies the hash-pinned upstream archive, every original and added file, unchanged upstream module manifests/license, patch hashes, and exact patch reconstruction. An unrelated edit or extra file fails this check. The downloaded archive is evidence input; its absence is an error, not a skipped check.

Run both regular and race MCP tests inside the SDK's original module and under the application's build list, with `GOWORK=off`, on Windows and Linux. `node scripts/check-go.mjs` executes both configurations. Application-boundary and independent raw-client tests remain required in addition to upstream tests.

For a security upgrade, inspect upstream changes, rebase only the approved patch files, regenerate identities deliberately, and rerun the same integrity, default-mode, compatibility and application-build-list gates. Upstream release activity does not establish that this patch can be removed. Do not update the copied upstream `go.mod` or `go.sum` to make a test pass.

## Go optimization round 1

The approved optimization contract adds a 10-second deadline to each compatibility SSE event, keepalive, and flush. Writes and their flush share a budget, cleared after completion so healthy streams can outlive it and the 15-second keepalive cadence. Failed replay/priming writes close the owning lease. The default SDK transport remains unchanged. Runtime shutdown starts a separate 30-second network-drain budget before joining MCP close, forces socket closure on expiry, and joins handlers before releasing borrowed resources. This is not a deadline for admitted database writes.

The application adapter separately limits original MCP request bodies to 4 MiB (HTTP 413 on overflow), before decoding, and allows at most six times that size after legacy JSON normalization. Authentication and Host/Origin/header precedence remain intact. Compatibility byte identities are regenerated from the same fixed upstream archive.
