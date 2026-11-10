-- name: JournalByID :one
SELECT j.journal_id,j.catalog_id,j.title,j.title_aliases_json,j.issns_json,j.issn,j.eissn,j.area,j.utd_rank,j.utd_rating,j.abs_rank,j.abs_rating,j.fms_rank,j.fms_rating,j.fmscn_rank,j.fmscn_rating,EXISTS(SELECT 1 FROM articles a WHERE a.journal_id=j.journal_id) FROM journals j WHERE j.journal_id=?;

-- name: IssueByID :one
SELECT i.issue_id,i.journal_id,i.publication_year,i.title,i.volume,i.number,i.date FROM issues i WHERE i.issue_id=?;

-- name: HasJournal :one
SELECT EXISTS(SELECT 1 FROM journals WHERE journal_id=?);

-- name: Areas :many
SELECT area,COUNT(*) FROM journals WHERE area IS NOT NULL AND area!='' GROUP BY area ORDER BY area ASC;

-- name: JournalOptions :many
SELECT journal_id,title FROM journals ORDER BY title ASC;

-- name: Years :many
SELECT publication_year,COUNT(DISTINCT issue_id),COUNT(DISTINCT journal_id) FROM issues WHERE publication_year IS NOT NULL GROUP BY publication_year ORDER BY publication_year DESC;
