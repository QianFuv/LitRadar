# LitRadar native SQLite open flag patch

Upstream: mattn/go-sqlite3 v1.14.52, commit `b0be46fa28d17ee0b65c79774ac0dad84b6db068`. The source copy and original MIT license remain under `../go-sqlite3`. The patch adds the opt-in `SQLiteDriver.NoFollow` flag and a regression test. Default flags, the amalgamation, VFS, engine and SQL semantics are unchanged.

Native Windows SQLite still follows a final symlink with NOFOLLOW, matching the frozen Rust SQLite observation; the application also validates final paths. Crossref workset ancestry/reparse/hard-link checks are a separate application policy, not a new global restriction on ordinary database paths. Tests retain the external target's exact bytes.

`upstream.json`, `patched.json` and `compatibility.patch` capture exact upstream, local and patch identities. Run `node tests/migration/run.mjs --phase sqlite-driver-integrity` from the root to reproduce the patch and reject any unlisted modification or new file.

Regular and race suites run with `sqlite_fts5` both in the original module and in the root build list, on Windows and Linux. These suites must be serialized across configurations because upstream tests use fixed temporary filenames in their working directory. Application tests additionally cover native Simple on each physical connection, reconnects, backups, transactions, URI conversion and owned paths.

An upstream/security upgrade requires rebase and fresh integrity/native regression proofs. Preserve the exact upstream `go.mod`, `go.sum`, license and amalgamation. Do not silently expand this into a native VFS or engine fork.
