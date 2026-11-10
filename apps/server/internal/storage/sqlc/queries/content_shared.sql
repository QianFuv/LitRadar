-- name: JournalProjection :one
SELECT title, area FROM journals WHERE journal_id = ?1;

-- name: UpsertJournal :execresult

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
        fmscn_rating = excluded.fmscn_rating;

-- name: ClaimArticleAlias :one
INSERT INTO article_identity_keys (identity_kind, identity_value, article_id)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(identity_kind, identity_value) DO UPDATE SET
                     article_id = article_identity_keys.article_id
                 RETURNING article_id;

