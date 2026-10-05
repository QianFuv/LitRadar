# Migrating an existing deployment to Go

The Go `litradar` executable replaces the first-party Rust application. Public
commands, HTTP/MCP contracts, persisted identities and supported storage formats
remain compatible. The static frontend still uses Node.js at build time. SQLite,
the Simple tokenizer, Poppler and the official Obscura executable remain native
dependencies. Obscura v0.2.4 release binaries, including its worker, are used under
the approved helper supply-chain review waiver; that waiver is not a security
audit pass.

## Release and storage inventory

Record the exact old executable/image, new image digest, platform, CLI arguments,
environment configuration and actual mounted paths. Retain the matching old
executable and required native libraries for a tested binary rollback. A tag alone
does not identify the tested image.

Inventory the complete deployment state, including:

- Auth SQLite, its WAL/SHM or journal sidecars, and any explicit `--auth-db` path.
- Content indexes, catalog control databases, the batch database, and provider
  worksets with their ownership files and exact checkpoints.
- Current and historical publication files in `data/push_state`, folder push
  state, legacy JSON import sources and CFP capture directories.
- Actual `data/meta` files, managed metadata state, and the release's bundled Meta
  assets. Preserve customized bytes and trailing newlines.
- Maintenance markers and staging/rollback directories, including malformed or
  apparently empty markers. Their presence can intentionally block startup.
- The matching secret key, inventoried and protected separately from data and
  backups. Preserve old/new key associations if rotation occurred.

Include external paths and bind-mounted directories even when they are outside
the project root. Never copy only `auth.sqlite` and assume that this is a complete
application snapshot. The built-in backup has explicit selected groups; it does
not include every control/workset/effect ledger or the deployment key.

## Before the first Go write

1. Stop incoming writes and manual job admission. Stop service instances,
   schedulers, dispatchers, CLI workers and their descendants. Verify there is no
   remaining writer against any inventoried path.
2. Preserve resumable and Unknown state; do not mark it successful or delete its
   leases merely to make the switch look clean. Resolve maintenance interruption
   using its existing recovery procedure before normal startup.
3. With all writers stopped, take one consistent whole-volume snapshot plus a
   separately protected matching key. Include SQLite sidecars. Record file paths,
   sizes and hashes and the release/bundle identity; protect this inventory from
   concurrent changes.
4. Rehearse on a disposable copy with the exact candidate image and the deployment's
   path overrides. Verify login, token/session behavior, search, secrets,
   publications and the relevant pending job categories. Use synthetic upstreams
   for external-effect testing.
5. Start exactly one Go writer against the live state when the deployment change
   itself is authorized. Check readiness and persisted task outcomes before
   reopening admission. Normal startup upgrades supported v4/v5 content layouts;
   the explicit storage optimizer intentionally refuses those old layouts.

These repository rehearsals do not perform a production cutover.

## Binary rollback after Go has written

Stop admission and every Go process first. Snapshot the **latest complete state**
and matching key, then start the retained compatible Rust binary against a copy
before selecting it for the live deployment. Keep current control/worksets,
prepared publication bytes, notification attempt IDs, acknowledgment records,
dedupe protection and Unknown outcomes together.

The rollback target is the latest state, not the pre-migration backup. A schema
number or successful `secrets verify` alone does not prove all state categories
are safe. The automated matrix combines actual-image handover with independent
historical reader/writer and crash-recovery tests. Pre-existing unsupported future
schemas and interrupted maintenance states must refuse without destructive
conversion. Newly emitted legitimate Go state must remain compatible.

Secret rotation changes which separately stored key belongs to that state.
Verify the new key with both binaries; keep the old snapshot paired with its old
key. Never put a plaintext key into an application backup to simplify rollback.

## Disaster recovery from an older backup

Restoring an older snapshot can lose newer data and external-effect evidence.
Keep admission stopped. First quarantine a complete copy of the current state
and its key, including receiver/message records needed to reconcile delivery.
Verify the backup manifest and selected groups before running the documented
`admin backup restore --confirm-restore` operation on a disposable copy.

Version 1 backups leave metadata untouched. Version 2 includes metadata.
Unselected index/push groups remain untouched; selected groups replace their
documented contents. Restore can therefore combine old auth/content with newer
catalog control, batch state and worksets. The application does not claim to
detect and repair every such mixed epoch automatically.

Preserve both epochs and their manifests. Compare checkpoint/anchor ownership,
batch fingerprints, prepared publication bytes and notification attempts before
deciding which jobs can resume. Missing individual ledgers are not permission to
re-send. An Unknown delivery means the receiver may already have accepted it;
review receiver evidence and use the existing explicit acknowledgment flow.
Acknowledgment records a new attempt and audit event while preserving the old
ambiguity and article-level dedupe protection. It does not erase evidence or
promise a second external send.

## Rehearsal commands and evidence

On the Windows x64 migration host with Docker available, D-drive build caches
configured and the frozen baseline archive installed:

```powershell
. 'D:\BuildCache\use-build-storage.ps1'
node tests/migration/run.mjs --phase cutover
node tests/migration/run.mjs --phase rollback
```

The runners resolve `litradar:go-test-amd64` to its exact image digest. They use
marker-protected synthetic roots, the preserved original Rust fixture/executable
and explicitly identified observers. They never rebuild a historical oracle from
the candidate implementation. Required binary hashes are recorded in
`tests/data/migration/oracle-win32-x64.json` and
`tests/migration/cutover/inputs.json`; missing archives fail the check.

Cutover includes a positive real execute-mode send, response loss, post-send local
commit failure, a process kill while sending, restart/no-replay, wrong CA and wrong
hostname controls, plus owner acknowledgment and atomic audit failure. HTTPS
uses the production hostname/path/SNI inside an isolated internal Docker network,
a separately mounted test CA and fake credentials. There is no external route,
host relay, TLS verification bypass or production test endpoint. The one-hour
production delivery lease is unchanged; the killed-process rehearsal explicitly
expires lease timestamps only after process death in its disposable database.

Live state is observed through HTTP. Windows SQLite observations and controlled
SQL faults happen only after the Linux container has stopped, avoiding concurrent
cross-platform WAL access. Inventory reads, semantic snapshots, exact integer
identities and current/history publication hashes are retained with each receipt.

Receipts are written to `output/migration/cutover/` and
`output/migration/rollback/`. A failed receipt is not a pass, and a targeted debug
run does not close the full phase. Disposable roots are retained for diagnosis;
containers and networks owned by the runner are removed after completion/failure.
Process-crash recovery evidence does not assert zero loss after host power failure.
