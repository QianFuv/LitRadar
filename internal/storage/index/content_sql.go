package index

const journalProjectionSql = `SELECT title, area FROM journals WHERE journal_id = ?1`

const journalUpsertSql = `
    INSERT INTO journals (
        journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area,
        utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating,
        fmscn_rank, fmscn_rating
    ) VALUES (
        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
    )
    ON CONFLICT(journal_id) DO UPDATE SET
        catalog_id = excluded.catalog_id,
        title = excluded.title,
        title_aliases_json = excluded.title_aliases_json,
        issns_json = excluded.issns_json,
        issn = excluded.issn,
        eissn = excluded.eissn,
        area = excluded.area,
        utd_rank = excluded.utd_rank,
        utd_rating = excluded.utd_rating,
        abs_rank = excluded.abs_rank,
        abs_rating = excluded.abs_rating,
        fms_rank = excluded.fms_rank,
        fms_rating = excluded.fms_rating,
        fmscn_rank = excluded.fmscn_rank,
        fmscn_rating = excluded.fmscn_rating`

const issueUpsertSql = `INSERT INTO issues (
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
                     date = COALESCE(excluded.date, issues.date)`

const identityLookupSql = `SELECT article_id FROM article_identity_keys
                 WHERE identity_kind = ?1 AND identity_value = ?2`

const articleLookupSql = `SELECT
                     j.catalog_id, a.title, a.publication_year, a.date, i.title, i.volume, i.number,
                     a.authors_json, a.start_page, a.end_page, a.abstract_text, a.doi, a.pmid,
                     a.open_access, a.in_press, a.issue_id
                 FROM articles AS a
                 JOIN journals AS j ON j.journal_id = a.journal_id
                 LEFT JOIN issues AS i ON i.issue_id = a.issue_id
                 WHERE a.article_id = ?1`

const retractionLookupSql = `SELECT retraction_doi FROM article_retraction_dois
                 WHERE article_id = ?1 ORDER BY retraction_doi`

const articleUpsertSql = `INSERT INTO articles (
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
                     in_press = excluded.in_press`

const retractionDeleteSql = `DELETE FROM article_retraction_dois WHERE article_id = ?1`

const retractionInsertSql = `INSERT INTO article_retraction_dois (article_id, retraction_doi)
                 VALUES (?1, ?2)`

const listingUpsertSql = `INSERT INTO article_listing (
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
                     area = excluded.area`

const searchDeleteSql = `DELETE FROM article_search WHERE rowid = ?1`

const searchInsertSql = `INSERT INTO article_search (
                     rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
                 ) VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)`

const identityClaimSql = `WITH prior(owner) AS MATERIALIZED (
                     SELECT (
                         SELECT article_id
                         FROM article_identity_keys
                         WHERE identity_kind = ?1 AND identity_value = ?2
                     )
                 )
                 INSERT INTO article_identity_keys (identity_kind, identity_value, article_id)
                 SELECT ?1, ?2, ?3
                 FROM prior
                 WHERE TRUE
                 ON CONFLICT(identity_kind, identity_value) DO UPDATE SET
                     article_id = article_identity_keys.article_id
                 RETURNING article_id, (SELECT owner IS NULL FROM prior)`

const changeEventInsertSql = `INSERT OR IGNORE INTO article_change_events (
                     content_revision, article_id, change_kind, journal_id, issue_id, in_press,
                     created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)`
