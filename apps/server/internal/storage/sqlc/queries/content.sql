-- name: ContentChanges :many
SELECT event_id,content_revision,article_id,change_kind,journal_id,issue_id,in_press,created_at FROM article_change_events WHERE event_id>?1 ORDER BY event_id LIMIT ?2;

-- name: AcknowledgeChanges :execresult
DELETE FROM article_change_events WHERE event_id<=?1;

-- name: DiscardChanges :execresult
DELETE FROM article_change_events;

-- name: JournalKeyOwner :one
SELECT canonical_catalog_id FROM journal_identity_keys WHERE identity_kind=?1 AND identity_value=?2;

-- name: InsertJournalKey :execresult
INSERT INTO journal_identity_keys(identity_kind,identity_value,canonical_catalog_id) VALUES(?1,?2,?3);

-- name: JournalID :one
SELECT journal_id FROM journals WHERE catalog_id=?1;

-- name: JournalCatalogID :one
SELECT catalog_id FROM journals WHERE journal_id=?1;

-- name: HasJournalCatalog :one
SELECT EXISTS(SELECT 1 FROM journals WHERE catalog_id=?1);

-- name: DeleteJournalKeys :execresult
DELETE FROM journal_identity_keys WHERE canonical_catalog_id=?1;

-- name: DeleteEmptyJournal :execresult
DELETE FROM journals WHERE journal_id=?1 AND catalog_id=?2;

-- name: UpdateListingArea :execresult
UPDATE article_listing SET area=?1 WHERE journal_id=?2;

-- name: JournalProjections :many
SELECT articles.article_id,articles.title,articles.abstract_text,articles.doi,articles.pmid,COALESCE((SELECT group_concat(CASE authors.type WHEN 'object' THEN json_extract(authors.value,'$.display_name') ELSE CAST(authors.value AS TEXT) END,'; ') FROM json_each(articles.authors_json) AS authors),'') FROM articles WHERE articles.journal_id=?1 ORDER BY articles.article_id;

-- name: UpdateIssueDate :execresult
UPDATE issues SET date=COALESCE((SELECT articles.date FROM articles WHERE articles.issue_id=?1 AND articles.date IS NOT NULL ORDER BY length(articles.date) DESC,articles.date LIMIT 1),issues.date) WHERE issues.issue_id=?1;

-- name: HasStoredArticleIdentity :one
SELECT EXISTS(
        SELECT 1 FROM article_identity_keys WHERE
        (article_id=?1 AND identity_kind IN ('doi','pmid')) OR
        (article_id=?2 AND ((identity_kind='doi' AND identity_value<>?3) OR
                           (identity_kind='pmid' AND identity_value<>?4))));

-- name: DeleteEmptyIssue :execresult
DELETE FROM issues WHERE issues.issue_id=?1 AND NOT EXISTS(SELECT 1 FROM articles WHERE articles.issue_id=?1);

