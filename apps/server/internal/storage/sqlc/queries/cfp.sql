-- name: InsertNotices :execresult
INSERT INTO cfp_notices(journal_key,notice_key,source_key,source_json,normalized_json,display_order) VALUES(?,?,?,?,?,?);

-- name: InsertJournal :execresult
INSERT INTO cfp_journals(journal_key,title,checked_on,source_url,source_statement) VALUES(?,?,?,?,?);

-- name: InsertAlias :execresult
INSERT INTO cfp_journal_aliases(catalog_id,journal_key) VALUES(?,?);

-- name: InsertSourceJournal :execresult
INSERT INTO cfp_source_journals(source_key,journal_key) VALUES(?,?);

-- name: SeedHash :one
SELECT content_hash FROM cfp_seed_imports WHERE seed_id=?;

-- name: InsertSource :execresult
INSERT INTO cfp_sources(source_key,url,parser_version,capture,capture_format,content_hash,status) VALUES(?,?,?,?,'seed_json',?,'snapshot');

-- name: RecordSeed :execresult
INSERT INTO cfp_seed_imports(seed_id,format_version,content_hash,parser_version,journal_count,notice_count) VALUES(?,1,?,?,?,?);

-- name: BeginRefresh :one
UPDATE cfp_sources SET generation=generation+1,lease_expires_at=?2,last_attempt=?3,last_error=NULL,status='refreshing' WHERE source_key=?1 RETURNING generation;

-- name: FailRefresh :execresult
UPDATE cfp_sources SET status=?3,last_error=?4,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at IS NOT NULL;

-- name: PublishSource :execresult
UPDATE cfp_sources SET capture=?3,capture_format=?4,content_hash=?5,url=?6,parser_version=?7,config_version=?8,last_success=CASE WHEN ?10 IS NULL THEN ?9 ELSE last_success END,last_error=?10,status=CASE WHEN ?10 IS NULL THEN 'success' ELSE 'failed' END,revision=revision+1,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at>=?9;

-- name: DeleteNotices :execresult
DELETE FROM cfp_notices WHERE source_key=? AND journal_key=?;

-- name: UpdateJournal :execresult
UPDATE cfp_journals SET checked_on=?2,source_url=?3,source_statement=?4 WHERE journal_key=?1;

-- name: Originals :many
SELECT source_json FROM cfp_notices WHERE source_key=? ORDER BY display_order,notice_key;

-- name: SourceJournals :many
SELECT journal_key FROM cfp_source_journals WHERE source_key=?;

-- name: JournalAliases :many
SELECT catalog_id FROM cfp_journal_aliases WHERE journal_key=?;
