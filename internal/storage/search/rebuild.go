package search

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
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
		var name string
		if isCanonical {
			if bytes.HasPrefix(bytes.TrimSpace(author), []byte("[")) {
				var fields []json.RawMessage
				if json.Unmarshal(author, &fields) != nil || len(fields) != 1 || !bytes.HasPrefix(bytes.TrimSpace(fields[0]), []byte(`"`)) || json.Unmarshal(fields[0], &name) != nil {
					return nil, invalid
				}
				result = append(result, name)
				continue
			}
			decoder := json.NewDecoder(bytes.NewReader(author))
			opening, err := decoder.Token()
			if err != nil || opening != json.Delim('{') || !decoder.More() {
				return nil, invalid
			}
			key, err := decoder.Token()
			if err != nil || key != "display_name" {
				return nil, invalid
			}
			var value json.RawMessage
			if err := decoder.Decode(&value); err != nil || !bytes.HasPrefix(bytes.TrimSpace(value), []byte(`"`)) || json.Unmarshal(value, &name) != nil || decoder.More() {
				return nil, invalid
			}
		} else if !bytes.HasPrefix(bytes.TrimSpace(author), []byte(`"`)) || json.Unmarshal(author, &name) != nil {
			return nil, invalid
		}
		result = append(result, name)
	}
	return result, nil
}

// UsesSimple inspects the actual table declaration without initializing the virtual table.
func UsesSimple(ctx context.Context, connection *sql.Conn) (bool, error) {
	var declaration storage.Text
	err := connection.QueryRowContext(ctx, "SELECT sql FROM sqlite_schema WHERE type='table' AND name='article_search'").Scan(&declaration)
	if errors.Is(err, sql.ErrNoRows) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return strings.Contains(storage.CompactSchema(string(declaration)), "tokenize='simple0'"), nil
}

// Rebuild streams canonical rows into search inside the caller's existing transaction and write exclusion.
func Rebuild(ctx context.Context, connection *sql.Conn) error {
	usesSimple, err := UsesSimple(ctx, connection)
	if err != nil {
		return err
	}
	if usesSimple {
		if err := storage.LoadSimple(connection); err != nil {
			return err
		}
	}
	if _, err := connection.ExecContext(ctx, "DELETE FROM article_search"); err != nil {
		return err
	}
	rows, err := connection.QueryContext(ctx, `SELECT a.article_id,a.title,COALESCE(a.abstract_text,''),COALESCE(a.doi,''),COALESCE(a.pmid,''),a.authors_json,j.title FROM articles a JOIN journals j ON j.journal_id=a.journal_id ORDER BY a.article_id`)
	if err != nil {
		return err
	}
	defer rows.Close()
	insert, err := connection.PrepareContext(ctx, `INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(?1,?1,?2,?3,?4,?5,?6,?7)`)
	if err != nil {
		return err
	}
	defer insert.Close()
	for rows.Next() {
		var id int64
		var title, abstract, doi, pmid, authors, journal storage.Text
		if err := rows.Scan(&id, &title, &abstract, &doi, &pmid, &authors, &journal); err != nil {
			return err
		}
		names, err := DecodeAuthorNames(string(authors))
		if err != nil {
			return err
		}
		if _, err := insert.ExecContext(ctx, id, PrepareText(string(title), usesSimple), PrepareText(string(abstract), usesSimple), PrepareText(string(doi), usesSimple), PrepareText(string(pmid), usesSimple), PrepareText(strings.Join(names, "; "), usesSimple), PrepareText(string(journal), usesSimple)); err != nil {
			return err
		}
	}
	return rows.Err()
}
