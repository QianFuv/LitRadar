# LitRadar native SQLite open and migration preflight patches

Upstream: mattn/go-sqlite3 v1.14.52, commit `b0be46fa28d17ee0b65c79774ac0dad84b6db068`. The source copy and original MIT license remain under `../go-sqlite3`. The patch adds a direct, per-connection `SQLiteConn.RegisterSimple` initializer and opt-in `SQLiteDriver.NoFollow` and `SQLiteDriver.DeferSynchronous` flags and regression tests. Default flags, the amalgamation, VFS, engine and SQL semantics are unchanged.

`DeferSynchronous` skips only the initial synchronous pragma, allowing migration to query `user_version` on the same connection before schema parsing. Current or future databases, including those with malformed schema, retain the version-first short circuit. The dedicated migration caller enables FK/WAL/NORMAL explicitly after accepting an older version. Ordinary connections keep upstream's NORMAL default. This is not permission to perform writes without completing connection policy.

Native Windows SQLite still follows a final symlink with NOFOLLOW; the application also validates final paths. Crossref workset ancestry/reparse/hard-link checks are a separate application policy, not a new global restriction on ordinary database paths. Tests retain the external target's exact bytes.

`upstream.json`, `patched.json` and `compatibility.patch` capture exact upstream, local and patch identities. Run `node scripts/verify-go-dependency.mjs go-sqlite3` from the root to reproduce the patch and reject any unlisted modification or new file.

Regular and race suites run with `sqlite_fts5` both in the original module and in the root build list, on Windows and Linux. These suites must be serialized across configurations because upstream tests use fixed temporary filenames in their working directory. Application tests additionally cover native Simple on each physical connection, reconnects, backups, transactions, URI conversion and owned paths.

An upstream/security upgrade requires rebase and fresh integrity/native regression proofs. Preserve the exact upstream `go.mod`, `go.sum`, license and amalgamation. Do not silently expand this into a native VFS or engine fork.

The direct initializer links the archive from `target/simple-tokenizer/libsimple.a`, compiled with `SQLITE_CORE` against the unchanged driver headers. It never enables SQL extension loading or registers a global automatic extension. Native initialization status and allocated error messages are handled explicitly, including closed connections and builds without FTS5. The source, compiler and cache inputs are documented in `../simple-static/README.md`.
