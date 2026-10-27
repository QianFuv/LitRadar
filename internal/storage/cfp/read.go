package cfp

import (
	"context"
	"database/sql"
	"encoding/json"
	"slices"
	"strings"

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
// Its retained connection is read-only: every writer, including this repository's
// pooled writes and other processes, changes its connection-local data_version.
func (repository *Repository) LoadJournals(ctx context.Context) (result []JournalSnapshot, resultError error) {
	defer func() { resultError = databaseError(resultError) }()
	repository.snapshotMutex.Lock()
	defer repository.snapshotMutex.Unlock()
	if repository.snapshotConnection == nil {
		connection, err := repository.database.Conn(ctx)
		if err != nil {
			return nil, err
		}
		repository.snapshotConnection = connection
	}
	var version sqlite.Integer
	var snapshots []JournalSnapshot
	err := connectionTransaction(ctx, repository.snapshotConnection, false, func(connection *sql.Conn) error {
		if err := connection.QueryRowContext(ctx, `PRAGMA main.data_version`).Scan(&version); err != nil {
			return err
		}
		if repository.snapshot != nil && repository.snapshotVersion == int64(version) {
			snapshots = repository.snapshot
			return nil
		}
		var err error
		snapshots, err = readJournals(ctx, connection, nil)
		return err
	})
	if err != nil {
		repository.clearSnapshot()
		return nil, err
	}
	repository.snapshot, repository.snapshotVersion = snapshots, int64(version)
	return cloneSnapshots(snapshots), nil
}

// LoadJournalsForCatalogIds reads every matching owner, including conflicts, in one snapshot.
func (repository *Repository) LoadJournalsForCatalogIds(ctx context.Context, catalogIds []string) ([]JournalSnapshot, error) {
	if len(catalogIds) == 0 {
		return []JournalSnapshot{}, nil
	}
	return repository.loadJournals(ctx, catalogIds)
}

// cloneSnapshots gives each caller independent slices and optional metadata pointers.
func cloneSnapshots(snapshots []JournalSnapshot) []JournalSnapshot {
	result := slices.Clone(snapshots)
	for index := range result {
		journal := &result[index]
		journal.CatalogIds = slices.Clone(journal.CatalogIds)
		if journal.SourceUrl != nil {
			journal.SourceUrl = new(*journal.SourceUrl)
		}
		if journal.SourceStatement != nil {
			journal.SourceStatement = new(*journal.SourceStatement)
		}
		journal.Notices = slices.Clone(journal.Notices)
		for index, notice := range journal.Notices {
			journal.Notices[index] = cloneNotice(notice)
		}
		journal.Sources = slices.Clone(journal.Sources)
		for index := range journal.Sources {
			source := &journal.Sources[index]
			if source.LastAttempt != nil {
				source.LastAttempt = new(*source.LastAttempt)
			}
			if source.LastSuccess != nil {
				source.LastSuccess = new(*source.LastSuccess)
			}
			if source.LastError != nil {
				source.LastError = new(*source.LastError)
			}
			if source.LeaseExpiresAt != nil {
				source.LeaseExpiresAt = new(*source.LeaseExpiresAt)
			}
		}
	}
	return result
}

// decodeNotice retains the persisted payload validation and error classification.
func decodeNotice(encoded string) (domain.Notice, error) {
	var notice domain.Notice
	if err := json.Unmarshal([]byte(encoded), &notice); err != nil {
		return domain.Notice{}, &PayloadError{Cause: err}
	}
	return notice, nil
}

// cloneNotice isolates all mutable fields from the immutable snapshot cache.
func cloneNotice(notice domain.Notice) domain.Notice {
	notice.Dates = slices.Clone(notice.Dates)
	if notice.TopicYear != nil {
		notice.TopicYear = new(*notice.TopicYear)
	}
	if notice.TimeZone != nil {
		notice.TimeZone = new(*notice.TimeZone)
	}
	if notice.SourceStatus != nil {
		notice.SourceStatus = new(*notice.SourceStatus)
	}
	return notice
}

// loadJournals reads live rows on a pooled connection without consulting the catalog cache.
func (repository *Repository) loadJournals(ctx context.Context, catalogIds []string) (result []JournalSnapshot, resultError error) {
	err := repository.transaction(ctx, false, func(connection *sql.Conn) error {
		var err error
		result, err = readJournals(ctx, connection, catalogIds)
		return err
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

// readJournals preserves row order and alias ownership within the caller's read transaction.
func readJournals(ctx context.Context, connection *sql.Conn, catalogIds []string) ([]JournalSnapshot, error) {
	journals := map[string]*JournalSnapshot{}
	where := ""
	var args []any
	if catalogIds != nil {
		placeholders := make([]string, len(catalogIds))
		for index, alias := range catalogIds {
			placeholders[index] = "?"
			args = append(args, alias)
		}
		where = ` WHERE journal_key IN (SELECT journal_key FROM cfp_journal_aliases WHERE catalog_id IN (` + strings.Join(placeholders, ",") + `))`
	}
	err := func() error {
		if err := readRows(ctx, connection, `SELECT journal_key,title,checked_on,source_url,source_statement FROM cfp_journals`+where+` ORDER BY journal_key`, args, func(row *sql.Rows) error {
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
		if err := readRows(ctx, connection, `SELECT catalog_id,journal_key FROM cfp_journal_aliases`+where+` ORDER BY catalog_id`, args, func(row *sql.Rows) error {
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
		if err := readRows(ctx, connection, `SELECT journal_key,normalized_json FROM cfp_notices`+where+` ORDER BY journal_key,display_order,notice_key`, args, func(row *sql.Rows) error {
			var key, encoded sqlite.Text
			if err := row.Scan(&key, &encoded); err != nil {
				return err
			}
			if journal := journals[string(key)]; journal != nil {
				notice, err := decodeNotice(string(encoded))
				if err != nil {
					return err
				}
				journal.Notices = append(journal.Notices, notice)
			}
			return nil
		}); err != nil {
			return err
		}
		return readRows(ctx, connection, `SELECT j.journal_key,s.source_key,s.status,s.last_attempt,s.last_success,s.last_error,s.revision,s.lease_expires_at FROM cfp_source_journals j JOIN cfp_sources s USING(source_key)`+where+` ORDER BY j.journal_key,s.source_key`, args, func(row *sql.Rows) error {
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
	}()
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
