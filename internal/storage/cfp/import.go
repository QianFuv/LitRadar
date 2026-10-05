package cfp

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func insertNotices(ctx context.Context, connection *sql.Conn, key, sourceKey string, notices []preparedNotice) error {
	for order, pair := range notices {
		source, err := jsonvalue.EncodeJson(pair.source)
		if err != nil {
			return err
		}
		notice, err := jsonvalue.EncodeJson(pair.notice)
		if err != nil {
			return err
		}
		if _, err = connection.ExecContext(ctx, `INSERT INTO cfp_notices(journal_key,notice_key,source_key,source_json,normalized_json,display_order) VALUES(?,?,?,?,?,?)`, key, pair.notice.Id, sourceKey, source, notice, order); err != nil {
			return err
		}
	}
	return nil
}
func emptyFields(journal *preparedJournal) (*string, *string) {
	if journal.empty == nil {
		return nil, nil
	}
	return &journal.empty.SourceUrl, &journal.empty.SourceStatement
}
func insertJournal(ctx context.Context, connection *sql.Conn, key, sourceKey string, journal *preparedJournal) error {
	url, statement := emptyFields(journal)
	if _, err := connection.ExecContext(ctx, `INSERT INTO cfp_journals(journal_key,title,checked_on,source_url,source_statement) VALUES(?,?,?,?,?)`, key, journal.title, journal.checkedOn, url, statement); err != nil {
		return err
	}
	for _, alias := range journal.aliases {
		if _, err := connection.ExecContext(ctx, `INSERT INTO cfp_journal_aliases(catalog_id,journal_key) VALUES(?,?)`, alias, key); err != nil {
			return err
		}
	}
	if _, err := connection.ExecContext(ctx, `INSERT INTO cfp_source_journals(source_key,journal_key) VALUES(?,?)`, sourceKey, key); err != nil {
		return err
	}
	return insertNotices(ctx, connection, key, sourceKey, journal.notices)
}

// ImportSeed validates a byte-identical, immutable operator import identity.
func (repository *Repository) ImportSeed(ctx context.Context, seedId string, input []byte) (ImportResult, error) {
	seed, err := PrepareSeed(input)
	if err != nil {
		return ImportResult{}, err
	}
	return repository.ImportPrepared(ctx, seedId, seed)
}

// ImportPrepared rechecks the import marker under the write lock and never merges existing owners.
func (repository *Repository) ImportPrepared(ctx context.Context, seedId string, seed *PreparedSeed) (ImportResult, error) {
	result := ImportResult{Journals: uint64(len(seed.journals)), Notices: seed.notices, ContentHash: seed.digest}
	err := repository.transaction(ctx, true, func(connection *sql.Conn) error {
		var previous sqlite.Text
		err := connection.QueryRowContext(ctx, `SELECT content_hash FROM cfp_seed_imports WHERE seed_id=?`, seedId).Scan(&previous)
		if err == nil {
			if string(previous) != seed.digest {
				return invalid("CFP seed identity has different content; use a new import identity")
			}
			return nil
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		for _, key := range sortedKeys(seed.journals) {
			journal := seed.journals[key]
			sourceKey := "journal:" + key
			originals := []domain.Source{}
			for _, pair := range journal.notices {
				originals = append(originals, pair.source)
			}
			capture, err := jsonvalue.EncodeJson(originals)
			if err != nil {
				return err
			}
			sourceUrl := ""
			if journal.empty != nil {
				sourceUrl = journal.empty.SourceUrl
			} else if len(journal.notices) > 0 {
				sourceUrl = journal.notices[0].source.SourceUrl
			}
			if _, err = connection.ExecContext(ctx, `INSERT INTO cfp_sources(source_key,url,parser_version,capture,capture_format,content_hash,status) VALUES(?,?,?,?,'seed_json',?,'snapshot')`, sourceKey, sourceUrl, domain.ParserVersion, capture, digest([]byte(capture))); err != nil {
				return err
			}
			if err = insertJournal(ctx, connection, key, sourceKey, journal); err != nil {
				return err
			}
		}
		_, err = connection.ExecContext(ctx, `INSERT INTO cfp_seed_imports(seed_id,format_version,content_hash,parser_version,journal_count,notice_count) VALUES(?,1,?,?,?,?)`, seedId, seed.digest, domain.ParserVersion, result.Journals, result.Notices)
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
