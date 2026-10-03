package indexschema

import (
	"context"
	"database/sql"
	"fmt"
	"slices"
	"strconv"
	"strings"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// InvalidStructure identifies the exact structural contract without interpreting arbitrary database data.
type InvalidStructure struct{ Message string }

// Error identifies the structural requirement that the database did not satisfy.
func (failure InvalidStructure) Error() string { return failure.Message }

func quoteList(values []string, isSet bool) string {
	quoted := make([]string, len(values))
	for index, value := range values {
		quoted[index] = strconv.Quote(value)
	}
	if isSet {
		return "{" + strings.Join(quoted, ", ") + "}"
	}
	return "[" + strings.Join(quoted, ", ") + "]"
}

func names(ctx context.Context, connection *sql.Conn, statement string) ([]string, error) {
	rows, err := connection.QueryContext(ctx, statement)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []string{}
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			return nil, err
		}
		result = append(result, value)
	}
	return result, rows.Err()
}

// ValidateStructure checks only the inventories, column ordering and search storage enforced by Rust.
func ValidateStructure(ctx context.Context, connection *sql.Conn, version int) error {
	var declaration string
	err := connection.QueryRowContext(ctx, "SELECT sql FROM sqlite_schema WHERE type='table' AND name='article_search'").Scan(&declaration)
	if err != nil && err != sql.ErrNoRows {
		return err
	}
	if strings.Contains(storage.CompactSchema(declaration), "tokenize='simple0'") {
		if err := storage.LoadSimple(connection); err != nil {
			return err
		}
	}
	if version < 4 || version > Version {
		return InvalidStructure{fmt.Sprintf("unsupported content schema version %d", version)}
	}
	expected := []string{}
	for _, table := range columns {
		if table.name == "journal_identity_keys" && version < 5 || table.name == "article_retraction_dois" && version < 6 {
			continue
		}
		expected = append(expected, table.name)
	}
	slices.Sort(expected)
	actual, err := names(ctx, connection, "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'article_search_%' ORDER BY name")
	if err != nil {
		return err
	}
	if !slices.Equal(actual, expected) {
		return InvalidStructure{"table inventory mismatch: " + quoteList(actual, true)}
	}
	for _, table := range columns {
		if !slices.Contains(expected, table.name) {
			continue
		}
		expectedColumns := slices.Clone(table.columns)
		if table.name == "articles" && version < 6 {
			expectedColumns = append(expectedColumns, "retraction_doi")
		}
		actual, err := names(ctx, connection, "SELECT name FROM pragma_table_info('"+table.name+"') ORDER BY cid")
		if err != nil {
			return err
		}
		if !slices.Equal(actual, expectedColumns) {
			return InvalidStructure{"column inventory mismatch for " + table.name + ": " + quoteList(actual, false)}
		}
	}
	expectedIndexes := slices.Clone(indexes)
	expectedIndexes = slices.DeleteFunc(expectedIndexes, func(name string) bool {
		return name == "idx_journal_identity_keys_catalog" && version < 5 || name == "idx_article_retraction_dois_doi" && version < 6
	})
	if version < 8 {
		expectedIndexes = append(expectedIndexes, "idx_article_change_events_order")
	}
	slices.Sort(expectedIndexes)
	actual, err = names(ctx, connection, "SELECT name FROM sqlite_schema WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
	if err != nil {
		return err
	}
	if !slices.Equal(actual, expectedIndexes) {
		return InvalidStructure{"index inventory mismatch: " + quoteList(actual, true)}
	}
	var hasContent bool
	if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='article_search_content')").Scan(&hasContent); err != nil {
		return err
	}
	tokenizer := "unicode61remove_diacritics2"
	content := ""
	if version >= 7 {
		content = "content='',contentless_delete=1,"
	}
	if version >= 9 {
		tokenizer = "simple0"
	}
	expectedSearch := "createvirtualtablearticle_searchusingfts5(article_idunindexed,title,abstract_text,doi,pmid,authors,journal_title," + content + "tokenize='" + tokenizer + "')"
	if hasContent != (version <= 6) || storage.CompactSchema(declaration) != expectedSearch {
		return InvalidStructure{"article_search storage options do not match the declared schema version"}
	}
	return nil
}

// Validate adds the explicit migration's foreign-key check; ordinary preflight uses ValidateStructure.
func Validate(ctx context.Context, connection *sql.Conn, version int) error {
	if err := ValidateStructure(ctx, connection, version); err != nil {
		return err
	}
	if version >= 6 {
		var violations int64
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM pragma_foreign_key_check").Scan(&violations); err != nil {
			return err
		}
		if violations != 0 {
			return InvalidStructure{"foreign key check failed"}
		}
	}
	return nil
}
