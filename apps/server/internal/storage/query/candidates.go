package query

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"slices"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// CollectIssueArticleCounts snapshots canonical issue membership for notification change detection.
func CollectIssueArticleCounts(ctx context.Context, filename string) (map[string]int64, error) {
	return collectCounts(ctx, filename, "SELECT journal_id,issue_id,COUNT(*) FROM articles WHERE issue_id IS NOT NULL GROUP BY journal_id,issue_id", true)
}

// CollectInPressArticleCounts includes only issue-less articles whose flag is exactly one.
func CollectInPressArticleCounts(ctx context.Context, filename string) (map[string]int64, error) {
	return collectCounts(ctx, filename, "SELECT journal_id,COUNT(*) FROM articles WHERE issue_id IS NULL AND COALESCE(in_press,0)=1 GROUP BY journal_id", false)
}

func collectCounts(ctx context.Context, filename, statement string, hasIssue bool) (resultValue1 map[string]int64, resultError error) {
	database, err := storage.OpenPlain(filename)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	rows, err := database.QueryContext(ctx, statement)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := rows.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close rows: %w", closeError))
		}
	}()
	result := map[string]int64{}
	for rows.Next() {
		var journal, issue, count storage.Integer
		if hasIssue {
			err = rows.Scan(&journal, &issue, &count)
		} else {
			err = rows.Scan(&journal, &count)
		}
		if err != nil {
			return nil, err
		}
		key := strconv.FormatInt(int64(journal), 10)
		if hasIssue {
			key += ":" + strconv.FormatInt(int64(issue), 10)
		}
		result[key] = int64(count)
	}
	return result, rows.Err()
}

// FetchCandidatesForIssueKeys validates both key components and selects canonical issue identifiers.
func FetchCandidatesForIssueKeys(ctx context.Context, filename string, keys []string) ([]domain.ArticleCandidate, error) {
	ids := make([]int64, 0, len(keys))
	for _, key := range keys {
		journal, issue, hasSeparator := strings.Cut(key, ":")
		if !hasSeparator {
			return nil, ErrInvalidCursor
		}
		if _, err := strconv.ParseInt(journal, 10, 64); err != nil {
			return nil, ErrInvalidCursor
		}
		id, err := strconv.ParseInt(issue, 10, 64)
		if err != nil {
			return nil, ErrInvalidCursor
		}
		ids = append(ids, id)
	}
	return fetchCandidates(ctx, filename, "a.issue_id", ids, "")
}

// FetchCandidatesForInPressKeys selects visible issue-less in-press records by journal.
func FetchCandidatesForInPressKeys(ctx context.Context, filename string, keys []string) ([]domain.ArticleCandidate, error) {
	ids := make([]int64, 0, len(keys))
	for _, key := range keys {
		id, err := strconv.ParseInt(key, 10, 64)
		if err != nil {
			return nil, ErrInvalidCursor
		}
		ids = append(ids, id)
	}
	return fetchCandidates(ctx, filename, "a.journal_id", ids, "a.issue_id IS NULL AND COALESCE(a.in_press,0)=1 AND ")
}

// FetchCandidatesForArticleIds deduplicates selected identifiers and orders by canonical date and ID.
func FetchCandidatesForArticleIds(ctx context.Context, filename string, ids []int64) ([]domain.ArticleCandidate, error) {
	return fetchCandidates(ctx, filename, "a.article_id", slices.Clone(ids), "")
}

func fetchCandidates(ctx context.Context, filename, column string, ids []int64, prefix string) (resultValue1 []domain.ArticleCandidate, resultError error) {
	if len(ids) == 0 {
		return []domain.ArticleCandidate{}, nil
	}
	slices.Sort(ids)
	ids = slices.Compact(ids)
	database, err := storage.OpenPlain(filename)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	if len(ids) <= 500 {
		return collect(ctx, database, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.abstract_text,a.date,a.open_access,a.in_press,a.doi,j.title FROM articles a JOIN journals j ON j.journal_id=a.journal_id WHERE `+prefix+column+" IN ("+placeholders(len(ids))+") ORDER BY a.date DESC,a.article_id DESC", arguments(ids), candidateFromRow)
	}
	connection, err := database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	return largeCandidateRows(ctx, connection, column, ids, prefix)
}

func largeCandidateRows(ctx context.Context, connection *sql.Conn, column string, ids []int64, prefix string) (resultValue1 []domain.ArticleCandidate, resultError error) {
	if _, err := connection.ExecContext(ctx, "CREATE TEMP TABLE candidate_membership(id INTEGER PRIMARY KEY) WITHOUT ROWID"); err != nil {
		return nil, err
	}
	defer func() {
		if _, err := connection.ExecContext(context.Background(), "DROP TABLE temp.candidate_membership"); err != nil {
			slog.Warn("storage.temporary_table.drop_failed", "error", err)
		}
	}()
	statement, err := connection.PrepareContext(ctx, "INSERT INTO temp.candidate_membership(id) VALUES "+strings.TrimSuffix(strings.Repeat("(?),", 500), ","))
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := statement.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close statement: %w", closeError))
		}
	}()
	for len(ids) >= 500 {
		if _, err := statement.ExecContext(ctx, arguments(ids[:500])...); err != nil {
			return nil, err
		}
		ids = ids[500:]
	}
	if len(ids) > 0 {
		if _, err := connection.ExecContext(ctx, "INSERT INTO temp.candidate_membership(id) VALUES "+strings.TrimSuffix(strings.Repeat("(?),", len(ids)), ","), arguments(ids)...); err != nil {
			return nil, err
		}
	}
	return candidateRows(ctx, connection, column+" IN (SELECT id FROM temp.candidate_membership)", prefix, nil)
}

func candidateRows(ctx context.Context, database rowQuerier, membership, prefix string, values []any) ([]domain.ArticleCandidate, error) {
	return collect(ctx, database, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.abstract_text,a.date,a.open_access,a.in_press,a.doi,j.title FROM articles a JOIN journals j ON j.journal_id=a.journal_id WHERE `+prefix+membership+" ORDER BY a.date DESC,a.article_id DESC", values, candidateFromRow)
}

// candidateFromRow retains strict storage classes without allocating pointers for flattened nullable fields.
func candidateFromRow(row scanner) (domain.ArticleCandidate, error) {
	var id, journal storage.Integer
	var openAccess, inPress sql.Null[storage.Integer]
	var issue storage.OptionalInteger
	var title, journalTitle storage.Text
	var abstract sql.Null[storage.Text]
	var date, doi storage.OptionalText
	if err := row.Scan(&id, &journal, &issue, &title, &abstract, &date, &openAccess, &inPress, &doi, &journalTitle); err != nil {
		return domain.ArticleCandidate{}, err
	}
	return domain.ArticleCandidate{ArticleId: int64(id), JournalId: int64(journal), IssueId: issue.Value, Title: string(title), Abstract: string(abstract.V), Date: date.Value, JournalTitle: string(journalTitle), Doi: doi.Value, OpenAccess: openAccess.V != 0, InPress: inPress.V != 0}, nil
}
