// Package content constructs synthetic catalog databases for Go regression tests.
package content

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// Batch provides one independently authored page for transaction and worker tests.
func Batch() (domain.JournalCatalogEntry, domain.ProviderBatch) {
	catalog := domain.JournalCatalogEntry{CatalogId: "example", Title: "Synthetic Journal", CatalogAliases: []string{}, AllIssns: []string{}, TitleAliases: []string{}}
	doi := "10.1234/synthetic"
	batch := domain.ProviderBatch{CatalogId: catalog.CatalogId, Journal: domain.JournalDraft{CatalogId: catalog.CatalogId}, Issues: []domain.IssueDraft{}, Articles: []domain.ArticleDraft{{CatalogId: catalog.CatalogId, Title: "Synthetic Article", Doi: &doi, Authors: []domain.ArticleAuthorDraft{{DisplayName: "Synthetic Author"}}}}, Progress: domain.ProviderProgress{State: domain.Complete}}
	return catalog, batch
}

// Create writes a fresh database with explicit version boundaries and synthetic records.
func Create(filename string, version int) error {
	if version < 4 || version > 9 {
		return fmt.Errorf("unsupported test schema version %d", version)
	}
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		return err
	}
	database, err := storage.OpenMigration(filename)
	if err != nil {
		return err
	}
	defer database.Close()
	ctx := context.Background()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	return writeFixtureContent(ctx, connection, version)
}

// fixtureContentSchema selects the historical search layout before any schema writes.
func fixtureContentSchema(connection *sql.Conn, version int) (string, error) {
	schema := indexschema.ContentTables
	if version < 9 {
		schema = strings.ReplaceAll(schema, "tokenize = 'simple 0'", "tokenize = 'unicode61 remove_diacritics 2'")
	} else if err := storage.LoadSimple(connection); err != nil {
		return "", err
	}
	if version < 7 {
		schema = strings.ReplaceAll(schema, "        content = '',\n        contentless_delete = 1,\n", "")
	}
	return schema, nil
}

// writeFixtureContent creates the schema and publishes fixed seed records in the original order.
func writeFixtureContent(ctx context.Context, connection *sql.Conn, version int) error {
	schema, err := fixtureContentSchema(connection, version)
	if err != nil {
		return err
	}

	if _, err := connection.ExecContext(ctx, schema); err != nil {
		return err
	}
	statements := `INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json)
		VALUES(1,'journal-one','Journal One','[]','["1234-5679"]'),(2,'journal-two','Journal Two','[]','["2049-3630"]');
		INSERT INTO journal_identity_keys VALUES('catalog_id','journal-one','journal-one'),('catalog_id','journal-two','journal-two'),('issn','1234-5679','journal-one'),('issn','2049-3630','journal-two');
		INSERT INTO articles(article_id,journal_id,title,authors_json,date)
		VALUES(1001,1,'Synthetic first article','[{"display_name":"Author One"}]','2026-01-01'),(1012,2,'Synthetic second article','[{"display_name":"Author Two"}]','2026-01-02');
		INSERT INTO article_listing(article_id,journal_id,date) SELECT article_id,journal_id,date FROM articles;
		INSERT INTO article_search(rowid,article_id,title,authors,journal_title)
		SELECT article_id,article_id,articles.title,'Author',journals.title FROM articles JOIN journals USING(journal_id);`
	if version < 8 {
		statements += "CREATE INDEX idx_article_change_events_order ON article_change_events(event_id);"
	}
	if version < 6 {
		statements += "DROP TABLE article_retraction_dois; ALTER TABLE articles ADD COLUMN retraction_doi TEXT;"
	}
	if version < 5 {
		statements += "DROP TABLE journal_identity_keys;"
	}
	_, err = connection.ExecContext(ctx, statements+fmt.Sprintf("PRAGMA user_version=%d", version))
	return err
}
