package search

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/indexschema"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// ErrInvalidAuthors aborts a rebuild rather than publishing a partial search projection.
var ErrInvalidAuthors = errors.New("invalid article author JSON")

// DecodeAuthorNames accepts either complete canonical author objects or a complete legacy string array.
func DecodeAuthorNames(payload string) ([]string, error) {
	invalid := ErrInvalidAuthors
	if !jsonvalue.ValidJson(payload) || !strings.HasPrefix(strings.TrimSpace(payload), "[") {
		return nil, invalid
	}
	var authors []json.RawMessage
	if err := json.Unmarshal([]byte(payload), &authors); err != nil {
		return nil, invalid
	}
	result := make([]string, 0, len(authors))
	isCanonical := len(authors) > 0 && !bytes.HasPrefix(bytes.TrimSpace(authors[0]), []byte(`"`))
	for _, author := range authors {
		name, err := decodeAuthorName(author, isCanonical)
		if err != nil {
			return nil, invalid
		}
		result = append(result, name)
	}
	return result, nil
}

// Rebuild streams canonical rows into search inside the caller's existing transaction and write exclusion.
func Rebuild(ctx context.Context, connection *sql.Conn) (resultError error) {
	if err := ctx.Err(); err != nil {
		return err
	}
	if _, err := connection.ExecContext(ctx, "DELETE FROM article_search"); err != nil {
		return err
	}
	rows, err := connection.QueryContext(ctx, `SELECT a.article_id,a.title,COALESCE(a.abstract_text,''),COALESCE(a.doi,''),COALESCE(a.pmid,''),a.authors_json,j.title FROM articles a JOIN journals j ON j.journal_id=a.journal_id ORDER BY a.article_id`)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := rows.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close rows: %w", closeError))
		}
	}()
	insert, err := connection.PrepareContext(ctx, `INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(?1,?1,?2,?3,?4,?5,?6,?7)`)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := insert.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close insert: %w", closeError))
		}
	}()
	for rows.Next() {
		if err := insertSearchRow(ctx, rows, insert); err != nil {
			return err
		}
	}
	return rows.Err()
}

// decodeAuthorName applies the representation selected by the first author without permitting mixed strings.
func decodeAuthorName(author json.RawMessage, isCanonical bool) (string, error) {
	if !isCanonical {
		return decodeAuthorString(author)
	}
	if bytes.HasPrefix(bytes.TrimSpace(author), []byte("[")) {
		return decodePositionalAuthor(author)
	}
	return decodeObjectAuthor(author)
}

// decodeAuthorString accepts a JSON string without null coercion.
func decodeAuthorString(author json.RawMessage) (string, error) {
	var name string
	if !bytes.HasPrefix(bytes.TrimSpace(author), []byte(`"`)) || json.Unmarshal(author, &name) != nil {
		return "", ErrInvalidAuthors
	}
	return name, nil
}

// decodePositionalAuthor retains exact single-field arity and string admission.
func decodePositionalAuthor(author json.RawMessage) (string, error) {
	var fields []json.RawMessage
	var name string
	if json.Unmarshal(author, &fields) != nil || len(fields) != 1 || !bytes.HasPrefix(bytes.TrimSpace(fields[0]), []byte(`"`)) || json.Unmarshal(fields[0], &name) != nil {
		return "", ErrInvalidAuthors
	}
	return name, nil
}

// decodeObjectAuthor retains the original single display_name member token sequence.
func decodeObjectAuthor(author json.RawMessage) (string, error) {
	var name string
	decoder := json.NewDecoder(bytes.NewReader(author))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') || !decoder.More() {
		return "", ErrInvalidAuthors
	}
	key, err := decoder.Token()
	if err != nil || key != "display_name" {
		return "", ErrInvalidAuthors
	}
	var value json.RawMessage
	if err := decoder.Decode(&value); err != nil || !bytes.HasPrefix(bytes.TrimSpace(value), []byte(`"`)) || json.Unmarshal(value, &name) != nil || decoder.More() {
		return "", ErrInvalidAuthors
	}
	return name, nil
}

// insertSearchRow preserves typed scan, complete author decoding and one ordered projection insertion.
func insertSearchRow(ctx context.Context, rows *sql.Rows, insert *sql.Stmt) error {
	var id int64
	var title, abstract, doi, pmid, authors, journal storage.Text
	if err := rows.Scan(&id, &title, &abstract, &doi, &pmid, &authors, &journal); err != nil {
		return err
	}
	names, err := DecodeAuthorNames(string(authors))
	if err != nil {
		return err
	}
	if _, err := insert.ExecContext(ctx, id, PrepareText(string(title)), PrepareText(string(abstract)), PrepareText(string(doi)), PrepareText(string(pmid)), PrepareText(strings.Join(names, "; ")), PrepareText(string(journal))); err != nil {
		return err
	}
	return nil
}

// NeedsRebuild compares the persisted declaration without opening an obsolete virtual table.
func NeedsRebuild(ctx context.Context, connection *sql.Conn) (bool, error) {
	var declaration string
	if err := connection.QueryRowContext(ctx, "SELECT sql FROM sqlite_schema WHERE type='table' AND name='article_search'").Scan(&declaration); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return false, nil
		}
		return false, err
	}
	return storage.CompactSchema(declaration) != storage.CompactSchema(strings.TrimSuffix(indexschema.SearchTable, ";")), nil
}

// Ensure replaces obsolete search storage within the caller's transaction and write exclusion.
func Ensure(ctx context.Context, connection *sql.Conn) error {
	shouldRebuild, err := NeedsRebuild(ctx, connection)
	if err != nil || !shouldRebuild {
		return err
	}
	if _, err := connection.ExecContext(ctx, "DROP TABLE article_search;"+indexschema.SearchTable); err != nil {
		return err
	}
	return Rebuild(ctx, connection)
}
