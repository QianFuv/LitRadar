-- name: JournalByID :one
SELECT j.journal_id,j.catalog_id,j.title,j.title_aliases_json,j.issns_json,j.issn,j.eissn,j.area,j.utd_rank,j.utd_rating,j.abs_rank,j.abs_rating,j.fms_rank,j.fms_rating,j.fmscn_rank,j.fmscn_rating,EXISTS(SELECT 1 FROM articles a WHERE a.journal_id=j.journal_id) FROM journals j WHERE j.journal_id=?;

-- name: CountArticles :one
SELECT COUNT(*) FROM articles;

-- name: CountJournals :one
SELECT COUNT(*) FROM journals;

-- name: CountIssues :one
SELECT COUNT(*) FROM issues;

-- name: ArticleLocator :one
SELECT a.article_id,j.catalog_id,j.title AS journal_title,j.issns_json,a.title,a.publication_year,a.date,a.authors_json,i.volume,i.number AS issue_number,a.start_page,a.end_page,a.doi,a.pmid
FROM articles a JOIN journals j ON j.journal_id=a.journal_id LEFT JOIN issues i ON i.issue_id=a.issue_id WHERE a.article_id=?;

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
