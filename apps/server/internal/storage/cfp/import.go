package cfp

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"maps"
	"slices"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/cfpdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// insertNotices preserves source order while reusing the insert statement for multi-notice publications.
var queries cfpdb.Queries

func insertNotices(ctx context.Context, connection *sql.Conn, key, sourceKey string, notices []preparedNotice) (resultError error) {
	var statement *sql.Stmt
	if len(notices) > 1 {
		var err error
		statement, err = connection.PrepareContext(ctx, cfpdb.InsertNotices)
		if err != nil {
			return err
		}
		defer func() {
			if closeError := statement.Close(); closeError != nil {
				resultError = errors.Join(resultError, fmt.Errorf("close statement: %w", closeError))
			}
		}()
	}
	for order, pair := range notices {
		source, err := jsonvalue.EncodeJSON(pair.source)
		if err != nil {
			return err
		}
		notice, err := jsonvalue.EncodeJSON(pair.notice)
		if err != nil {
			return err
		}
		if statement == nil {
			_, err = queries.InsertNotices(ctx, connection, cfpdb.InsertNoticesParams{JournalKey: sqlite.Text(key), NoticeKey: sqlite.Text(pair.notice.ID), SourceKey: sqlite.Text(sourceKey), SourceJSON: sqlite.Text(source), NormalizedJSON: sqlite.Text(notice), DisplayOrder: sqlite.Integer(order)})
		} else {
			_, err = statement.ExecContext(ctx, key, pair.notice.ID, sourceKey, source, notice, order)
		}
		if err != nil {
			return err
		}
	}
	return nil
}
func emptyFields(journal *preparedJournal) (*string, *string) {
	if journal.empty == nil {
		return nil, nil
	}
	return &journal.empty.SourceURL, &journal.empty.SourceStatement
}
func insertJournal(ctx context.Context, connection *sql.Conn, key, sourceKey string, journal *preparedJournal) error {
	url, statement := emptyFields(journal)
	if _, err := queries.InsertJournal(ctx, connection, cfpdb.InsertJournalParams{JournalKey: sqlite.Text(key), Title: sqlite.Text(journal.title), CheckedOn: sqlite.Text(journal.checkedOn), SourceURL: (*sqlite.Text)(url), SourceStatement: (*sqlite.Text)(statement)}); err != nil {
		return err
	}
	for _, alias := range journal.aliases {
		if _, err := queries.InsertAlias(ctx, connection, cfpdb.InsertAliasParams{CatalogID: sqlite.Text(alias), JournalKey: sqlite.Text(key)}); err != nil {
			return err
		}
	}
	if _, err := queries.InsertSourceJournal(ctx, connection, cfpdb.InsertSourceJournalParams{SourceKey: sqlite.Text(sourceKey), JournalKey: sqlite.Text(key)}); err != nil {
		return err
	}
	return insertNotices(ctx, connection, key, sourceKey, journal.notices)
}

// ImportSeed validates a byte-identical, immutable operator import identity.
func (repository *Repository) ImportSeed(ctx context.Context, seedID string, input []byte) (ImportResult, error) {
	seed, err := PrepareSeed(input)
	if err != nil {
		return ImportResult{}, err
	}
	return repository.ImportPrepared(ctx, seedID, seed)
}

// ImportPrepared rechecks the import marker under the write lock and never merges existing owners.
func (repository *Repository) ImportPrepared(ctx context.Context, seedID string, seed *PreparedSeed) (ImportResult, error) {
	result := ImportResult{Journals: uint64(len(seed.journals)), Notices: seed.notices, ContentHash: seed.digest}
	err := repository.transaction(ctx, true, func(connection *sql.Conn) error {
		var previous sqlite.Text
		err := func() error {
			queryResult, err := queries.SeedHash(ctx, connection, cfpdb.SeedHashParams{SeedID: sqlite.Text(seedID)})
			if err != nil {
				return err
			}
			previous = queryResult
			return nil
		}()
		if err == nil {
			if string(previous) != seed.digest {
				return invalid("CFP seed identity has different content; use a new import identity")
			}
			return nil
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		for _, key := range slices.Sorted(maps.Keys(seed.journals)) {
			journal := seed.journals[key]
			sourceKey := "journal:" + key
			originals := []domain.Source{}
			for _, pair := range journal.notices {
				originals = append(originals, pair.source)
			}
			capture, err := jsonvalue.EncodeJSON(originals)
			if err != nil {
				return err
			}
			sourceURL := ""
			if journal.empty != nil {
				sourceURL = journal.empty.SourceURL
			} else if len(journal.notices) > 0 {
				sourceURL = journal.notices[0].source.SourceURL
			}
			if _, err = queries.InsertSource(ctx, connection, cfpdb.InsertSourceParams{SourceKey: sqlite.Text(sourceKey), URL: sqlite.Text(sourceURL), ParserVersion: sqlite.Integer(domain.ParserVersion), Capture: sqlite.Text(capture), ContentHash: sqlite.Text(digest([]byte(capture)))}); err != nil {
				return err
			}
			if err = insertJournal(ctx, connection, key, sourceKey, journal); err != nil {
				return err
			}
		}
		_, err = queries.RecordSeed(ctx, connection, cfpdb.RecordSeedParams{SeedID: sqlite.Text(seedID), ContentHash: sqlite.Text(seed.digest), ParserVersion: sqlite.Integer(domain.ParserVersion), JournalCount: sqlite.Integer(result.Journals), NoticeCount: sqlite.Integer(result.Notices)})
		if err == nil {
			result.DidImport = true
		}
		return err
	})
	if err != nil {
		return ImportResult{}, err
	}
	return result, nil
}
