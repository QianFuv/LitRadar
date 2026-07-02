# Rust Migration Contract Baseline

This document records the Python backend behavior that must remain stable while
the backend is migrated to Rust. The contract tests in `tests/contracts/` are the
executable source of truth for these details.

## API Contracts

- `article_id` and `journal_id` are serialized as decimal JSON strings, including
  values larger than JavaScript's safe integer range.
- Article listing keeps the existing `page` shape, `include_total=false` behavior,
  keyset cursor format, FTS filtering, and stored full-text redirect status.
- DB resolution preserves the current error split: missing databases return 404,
  while ambiguous omitted `db` values return 400.
- Browser login stores the raw session token only in the `ps_session` cookie.
  Authenticated profile responses do not expose the token value.
- Favorite bulk add returns `{"added": count}` while bulk remove and move return
  `{"count": count}`.
- CNKI session status responses expose only safe metadata such as status, token
  presence, expiration, and cookie names. Raw cookies and token values are not
  returned.
- CNKI full-text responses preserve PDF media type and `Content-Disposition`
  filename metadata when an exact provider match is available.

## Worker And State Contracts

- Scheduled task execution records `success` for zero exit status and
  `failed (<code>)` for non-zero shell command exit status.
- Notification state files keep `db_name`, `status`, `snapshot`, `run`,
  `delivery_dedupe`, and `updated_at` fields. Skipped runs keep pending issue and
  in-press keys in the active run state.
- Change manifests keep `run_id`, `generated_at`, `db_name`, `db_path`, changed
  groups, notifiable article IDs, backfill groups, and summary details. In-press
  additions are notifiable even when their article date is old.

## Index And External Source Contracts

- Stable integer IDs use the existing SHA-256-derived SQLite-safe integer
  conversion from `paper_scanner.shared.converters.to_int_stable`.
- Index API statistics and error summaries redact query secrets such as API keys,
  tokens, and mailto parameters before persistence or display.
- Contract fixtures under `tests/fixtures/contracts/recorded_http/` represent
  Crossref, OpenAlex, Semantic Scholar, CNKI, ZJLib, PushPlus, and OpenAI-compatible
  responses. Normal contract tests must use fixtures or fakes instead of live
  upstream services.

## Drift Policy

Rust implementations may improve internal module boundaries, but they must match
these observable contracts before any route, worker, indexer, or deployment
cutover. Any intentional contract change requires a separate approved plan update
before the Rust implementation is allowed to diverge.
