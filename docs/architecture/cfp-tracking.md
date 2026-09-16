# CFP tracking

The workspace `/?view=cfp-tracking` browses original journal calls by database and
maintained catalog identity. Rust owns acquisition, normalization, persistence,
identity matching and availability. The browser reads authenticated APIs and keeps
only display labels, formatting, filtering and selection state. `cfp_db` and
`cfp_journal` stay separate from the article-search database preference.

## Data ownership

The backend packages `crates/litradar-sources/assets/cfp-seed.json`: 467 journal
identities, 1,622 notices and 16 scoped publisher statements with no recorded calls.
The original-language payload is imported once into the existing `data/auth.sqlite`
business database. It is not included in browser JavaScript. Startup neither
requires article indexes nor overwrites a later source refresh.

Schema 18 adds `cfp_journals`, `cfp_journal_aliases`, `cfp_sources`,
`cfp_source_journals`, `cfp_notices` and `cfp_seed_imports`. Announcements are unique
within a journal; a shared multi-journal call is retained for each journal. Import
markers bind a versioned identity to its exact content digest. Malformed payloads,
conflicting aliases or existing journal ownership reject the whole import.

The standard central-database backup includes all CFP state. Before upgrading an
existing installation from schema 17, create and verify a normal backup. An older
binary cannot read schema 18; rollback restores the verified backup rather than
changing `PRAGMA user_version`.

## Backend operations

```text
litradar cfp refresh --project-root PATH --db english_journals.sqlite
litradar cfp refresh --project-root PATH --catalog-id issn-0304-4076
litradar cfp refresh --project-root PATH --all
litradar cfp refresh --project-root PATH --all --full-text --capture-dir OUTPUT_DIRECTORY
litradar cfp import --project-root PATH --input FILE
```

Refresh selection accepts exactly one database, maintained catalog ID/alias or
`--all`. The command reports successful, failed, unsupported and unattempted source
units. Failed or unattempted units cause a nonzero exit status. Import accepts the
backend seed format and is additive and immutable: it cannot replace an existing
journal's online snapshot. Reimporting identical bytes is idempotent. The bundled
seed uses its bundled identity; other inputs use a digest-derived operator identity.

`--full-text` revisits the stored announcements, including snapshot-only sources.
It matches their original titles, follows detail and PDF links with a bounded
depth, and retains complete paragraphs without a character limit. Springer
collection descriptions are read from the complete description container,
including text hidden by its Show more control. That publisher uses Obscura to
complete its public cookie handshake. Other sources try direct HTTP first and can
use the same supervised fallback. Four journal attempts can run concurrently,
with captures cached within each attempt. Springer descriptions are available at
DOMContentLoaded; their captured text is checked before publication.

This operation updates original scope, requirements and the verified detail URL;
it preserves existing titles and timeline semantics. Verified replacements and
unresolved originals are published together in one fenced transaction. Partial
results retain failed notices and the last complete-refresh timestamp, with an
explicit error explaining the remaining limitation. A completely failed attempt
does not replace the previous snapshot. Output distinguishes recovered text from
records actually updated. `--capture-dir` retains each journal's responses,
recovered fields and unresolved titles for inspection. It does not cause a later
run to silently reuse old captures.
An explicit `--resume-captures` revalidates the saved responses in that same capture
directory with the current parser, and fetches missing pages. This supports
resuming the same interrupted acquisition without repeating successful downloads.

`cfp-sources.json` records every retained acquisition unit, exact observed discovery
URL, journal identities, permitted host/path boundaries and adapter capability.
There are 106 configured discovery adapters: 65 Springer collection lists, 39
ScienceDirect CFP lists and 2 KeAi CFP sections. The other 361 units retain reviewed
snapshots and explicitly lack automatic discovery support. These counts describe
configured parser coverage, not a new successful live crawl of every publisher.

Refresh reads the actual discovery page and can discover newly added announcements.
It extracts literal card/section text and follows required registered detail links.
Wrong journal identity, conflicting titles, incomplete cards, pagination requiring
additional adaptation, challenges and empty responses do not establish an empty
journal. Filtered/preview lists and collections with independently reviewed detail
records retain older verified records. An explicit configured negative statement
can establish a scoped empty result; it never closes ordinary submissions by
inference. More complex HTML-to-PDF wrapper chains remain unadapted rather than
being published as complete announcements.

HTTP and subprocess work happens outside write transactions. Publication validates
all journal bindings, checks the current refresh generation and lease, and replaces
a complete source unit atomically. A failed or superseded attempt preserves the
last-good notices, captures and success timestamp. Interrupted leases appear as
failed/stale reads, not permanent in-progress states.

## Obscura and PDF helpers

The backend first tries direct HTTP. Connection failures, restricted statuses,
challenges and unrecognized JavaScript-dependent pages can use one Obscura fallback
per source, within the remaining source budget. No browser API invokes Obscura.
Executable resolution uses PATH by default, with these backend overrides:

- `LITRADAR_OBSCURA_PATH` or `--obscura-path PATH`
- `LITRADAR_PDFTOTEXT_PATH` or `--pdftotext-path PATH`

The Docker image includes `/usr/local/bin/obscura` and `/usr/bin/pdftotext` and
sets both backend path overrides. Obscura `0.2.2+litradar.1` is built from pinned
0.2.2 source with the rustls/webpki security update, native JavaScript/DOM support
and rendering/stealth support matching the locally used browser. The parallel
`scrape` worker is not packaged because CFP acquisition uses `fetch`. Chromium,
Node.js and runtime browser downloads are unnecessary. PDF extraction uses Debian's `poppler-utils`
and `poppler-data` character maps, including the CJK maps used by Chinese PDFs.
Both helpers run as the existing unprivileged service user; temporary captures
use `/tmp` without relaxing read-only root or noexec mount settings.

Obscura runs without a shell, with `--stealth`, a finite timeout, and a JSON `--eval`
envelope containing the final URL and original HTML. Exit zero is insufficient:
framing, URL, publisher identity, challenges and original call boundaries are still
validated. Private-network overrides are removed from the child environment;
Obscura retains its own default private-network protection. Rust independently
checks HTTP DNS results and each HTTP redirect before requesting it. Browser helper
final-URL validation is a result check; it does not claim visibility into the
helper's internal subresource/redirect implementation.

Directly linked text PDFs use `pdftotext` with UTF-8 output. The original PDF text
must contain the expected call title. Both helpers use owned process trees and
bounded temporary-file outputs; Windows processes are hidden. Remaining descendants
are terminated even when the leader exits first. Missing helpers, extraction errors
and image-only/unrecognized PDFs retain previous data.

Default limits are two concurrent sources, 90 seconds per source, 600 seconds per
batch, two HTTP attempts per page, five redirects, twelve required detail pages,
4 MiB per decoded page/helper result and 24 MiB per source capture. CLI timeout
options are capped at 600 seconds per source and 3,600 seconds per batch. A bounded
pool of at most eight OS DNS lookups lives outside the request runtime so an
uncancellable lookup cannot block request-client teardown. No recurring scheduler
job, browser refresh queue or admin refresh button is added by this migration.

## Original text and availability

`litradar-domain::cfp` normalizes literal fields without translation, generated
summaries or language-model calls. Missing scope and requirements remain empty.
Chinese/English date clauses retain their original text, stage, exclusivity and
optional status. Invalid or ambiguous timelines remain available as original text
with an uncertain state; missing dates do not imply perpetual acceptance.

Required abstract/proposal gates precede the full-paper gate. Optional workshops,
revisions, notifications, publication and registration dates do not reopen admission.
Explicit source closure, invitation-only and historical states are retained.
Source timezones are used when known; an unknown timezone remains conservative
around a cutoff. Date-dependent state is evaluated by the backend at query time.

## Read APIs

Both endpoints require the existing session cookie or bearer token:

| Endpoint | Behavior |
|---|---|
| `GET /api/cfp/journals?db=NAME&q=TEXT` | Complete lightweight metadata catalog, source coverage, state counts and freshness; optional name/ISSN/alias search |
| `GET /api/cfp/journals/{catalog_id}/notices?db=NAME&include_closed=false&limit=50&cursor=TOKEN` | Original notice page, authoritative state and entry deadline; maximum page size 200 |

The database filename is explicit and must identify a maintained metadata CSV.
Catalogs include journals without indexed articles. Valid unadapted members return
an empty 200 response; missing databases or members return 404. Cursor tokens use
authenticated encryption and bind database, canonical journal, filter, ordering,
metadata/source revision, position and a fixed evaluation instant. They expire after
15 minutes. Stale, altered or foreign tokens return 409; the page discards old pages
and reloads from the first cursor. Backend read failures show an error/retry state
without a bundled-data fallback.

## Verification

Rust fixtures compare all 1,622 imported notices with the prior original-text
contract at three fixed instants, and cover migration, aliases, concurrent import,
fenced publication, backup recovery, source boundaries, helper cleanup and DNS
teardown. API tests cover auth, catalogs without articles, cursor consistency,
refresh visibility and restart persistence. Frontend tests cover source-language
rendering, server-owned state, pagination, failed reads and late selection results.
Browser checks retain the second-row navigation geometry and mobile layout. The
marker-guarded full-stack fixture proves a real HTTP-source refresh changes the
already built page without rebuilding the frontend.
