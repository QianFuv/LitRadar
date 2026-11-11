package index

const searchDeleteSQL = `DELETE FROM article_search WHERE rowid = ?1`

const searchInsertSQL = `INSERT INTO article_search (
                     rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
                 ) VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)`

const identityClaimSQL = `WITH prior(owner) AS MATERIALIZED (
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
