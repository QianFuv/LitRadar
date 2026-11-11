-- name: UpsertIssue :execresult
INSERT INTO issues (
                     issue_id, journal_id, publication_year, title, volume, number, date
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(issue_id) DO UPDATE SET
                     publication_year = COALESCE(excluded.publication_year, issues.publication_year),
                     title = CASE
                         WHEN issues.title IS NULL THEN excluded.title
                         WHEN excluded.title IS NULL THEN issues.title
                         WHEN length(excluded.title) > length(issues.title) THEN excluded.title
                         WHEN length(excluded.title) < length(issues.title) THEN issues.title
                         ELSE MIN(issues.title, excluded.title)
                     END,
                     volume = CASE
                         WHEN issues.volume IS NULL THEN excluded.volume
                         WHEN excluded.volume IS NULL THEN issues.volume
                         ELSE MIN(issues.volume, excluded.volume)
                     END,
                     number = CASE
                         WHEN issues.number IS NULL THEN excluded.number
                         WHEN excluded.number IS NULL THEN issues.number
                         ELSE MIN(issues.number, excluded.number)
                     END,
                     date = COALESCE(excluded.date, issues.date);

-- name: IdentityOwner :one
SELECT article_id FROM article_identity_keys
                 WHERE identity_kind = ?1 AND identity_value = ?2;

-- name: Article :one
SELECT
                     j.catalog_id, a.title, a.publication_year, a.date, i.title, i.volume, i.number,
                     a.authors_json, a.start_page, a.end_page, a.abstract_text, a.doi, a.pmid,
                     a.open_access, a.in_press, a.issue_id
                 FROM articles AS a
                 JOIN journals AS j ON j.journal_id = a.journal_id
                 LEFT JOIN issues AS i ON i.issue_id = a.issue_id
                 WHERE a.article_id = ?1;

-- name: Retractions :many
SELECT retraction_doi FROM article_retraction_dois
                 WHERE article_id = ?1 ORDER BY retraction_doi;

-- name: UpsertArticle :execresult
INSERT INTO articles (
                     article_id, journal_id, issue_id, title, publication_year, date, authors_json,
                     start_page, end_page, abstract_text, doi, pmid, open_access, in_press
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                 )
                 ON CONFLICT(article_id) DO UPDATE SET
                     journal_id = excluded.journal_id,
                     issue_id = excluded.issue_id,
                     title = excluded.title,
                     publication_year = excluded.publication_year,
                     date = excluded.date,
                     authors_json = excluded.authors_json,
                     start_page = excluded.start_page,
                     end_page = excluded.end_page,
                     abstract_text = excluded.abstract_text,
                     doi = excluded.doi,
                     pmid = excluded.pmid,
                     open_access = excluded.open_access,
                     in_press = excluded.in_press;

-- name: DeleteRetractions :execresult
DELETE FROM article_retraction_dois WHERE article_id = ?1;

-- name: InsertRetraction :execresult
INSERT INTO article_retraction_dois (article_id, retraction_doi)
                 VALUES (?1, ?2);

-- name: UpsertListing :execresult
INSERT INTO article_listing (
                     article_id, journal_id, issue_id, publication_year, date, open_access,
                     in_press, doi, pmid, area
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(article_id) DO UPDATE SET
                     journal_id = excluded.journal_id,
                     issue_id = excluded.issue_id,
                     publication_year = excluded.publication_year,
                     date = excluded.date,
                     open_access = excluded.open_access,
                     in_press = excluded.in_press,
                     doi = excluded.doi,
                     pmid = excluded.pmid,
                     area = excluded.area;

-- name: InsertChangeEvent :execresult
INSERT OR IGNORE INTO article_change_events (
                     content_revision, article_id, change_kind, journal_id, issue_id, in_press,
                     created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);

