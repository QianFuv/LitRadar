package cfp

import (
	"context"
	"database/sql"
	"encoding/json"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// readRows closes each cursor before the next statement while retaining the caller's transaction.
func readRows(ctx context.Context, connection *sql.Conn, query string, args []any, scan func(*sql.Rows) error) error {
	rows, err := connection.QueryContext(ctx, query, args...)
	if err != nil {
		return err
	}
	defer rows.Close()
	for rows.Next() {
		if err = scan(rows); err != nil {
			return err
		}
	}
	return rows.Err()
}

// LoadJournals reads headers, aliases, notices and freshness under one consistent snapshot.
func (repository *Repository) LoadJournals(ctx context.Context) ([]JournalSnapshot, error) {
	journals := map[string]*JournalSnapshot{}
	err := repository.transaction(ctx, false, func(connection *sql.Conn) error {
		if err := readRows(ctx, connection, `SELECT journal_key,title,checked_on,source_url,source_statement FROM cfp_journals ORDER BY journal_key`, nil, func(row *sql.Rows) error {
			var key, title, checked sqlite.Text
			var url, statement sqlite.OptionalText
			if err := row.Scan(&key, &title, &checked, &url, &statement); err != nil {
				return err
			}
			journals[string(key)] = &JournalSnapshot{JournalKey: string(key), JournalTitle: string(title), CheckedOn: string(checked), SourceUrl: url.Value, SourceStatement: statement.Value, CatalogIds: []string{}, Notices: []domain.Notice{}, Sources: []SourceStatus{}}
			return nil
		}); err != nil {
			return err
		}
		if err := readRows(ctx, connection, `SELECT catalog_id,journal_key FROM cfp_journal_aliases ORDER BY catalog_id`, nil, func(row *sql.Rows) error {
			var alias, key sqlite.Text
			if err := row.Scan(&alias, &key); err != nil {
				return err
			}
			if journal := journals[string(key)]; journal != nil {
				journal.CatalogIds = append(journal.CatalogIds, string(alias))
			}
			return nil
		}); err != nil {
			return err
		}
		if err := readRows(ctx, connection, `SELECT journal_key,normalized_json FROM cfp_notices ORDER BY journal_key,display_order,notice_key`, nil, func(row *sql.Rows) error {
			var key, encoded sqlite.Text
			if err := row.Scan(&key, &encoded); err != nil {
				return err
			}
			if journal := journals[string(key)]; journal != nil {
				var notice domain.Notice
				if err := json.Unmarshal([]byte(encoded), &notice); err != nil {
					return &PayloadError{Cause: err}
				}
				journal.Notices = append(journal.Notices, notice)
			}
			return nil
		}); err != nil {
			return err
		}
		return readRows(ctx, connection, `SELECT j.journal_key,s.source_key,s.status,s.last_attempt,s.last_success,s.last_error,s.revision,s.lease_expires_at FROM cfp_source_journals j JOIN cfp_sources s USING(source_key) ORDER BY j.journal_key,s.source_key`, nil, func(row *sql.Rows) error {
			var key, source, status sqlite.Text
			var attempted, succeeded, expires sqlite.OptionalInteger
			var failure sqlite.OptionalText
			var revision sqlite.Integer
			if err := row.Scan(&key, &source, &status, &attempted, &succeeded, &failure, &revision, &expires); err != nil {
				return err
			}
			if journal := journals[string(key)]; journal != nil {
				journal.Sources = append(journal.Sources, SourceStatus{SourceKey: string(source), Status: string(status), LastAttempt: attempted.Value, LastSuccess: succeeded.Value, LastError: failure.Value, Revision: int64(revision), LeaseExpiresAt: expires.Value})
			}
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	result := make([]JournalSnapshot, 0, len(journals))
	for _, key := range sortedKeys(journals) {
		result = append(result, *journals[key])
	}
	return result, nil
}

// LoadOriginals returns last-good literal fragments in their persisted source order.
func (repository *Repository) LoadOriginals(ctx context.Context, sourceKey string) (result []domain.Source, resultError error) {
	defer func() { resultError = databaseError(resultError) }()
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	originals := []string{}
	err = readRows(ctx, connection, `SELECT source_json FROM cfp_notices WHERE source_key=? ORDER BY display_order,notice_key`, []any{sourceKey}, func(row *sql.Rows) error {
		var original sqlite.Text
		if err := row.Scan(&original); err != nil {
			return err
		}
		originals = append(originals, string(original))
		return nil
	})
	if err != nil {
		return nil, err
	}
	result = make([]domain.Source, 0, len(originals))
	for _, original := range originals {
		var source domain.Source
		if err = json.Unmarshal([]byte(original), &source); err != nil {
			return nil, &PayloadError{Cause: err}
		}
		result = append(result, source)
	}
	return result, nil
}
