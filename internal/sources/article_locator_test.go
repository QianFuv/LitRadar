package sources

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/query"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func locatorDatabase(t *testing.T) (config.Config, *sql.DB) {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := storage.Open(filepath.Join(configuration.IndexDir, "content.sqlite"), false, 1)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	_, err = database.Exec(`CREATE TABLE journals(journal_id INTEGER PRIMARY KEY,catalog_id TEXT,title TEXT,issns_json TEXT);CREATE TABLE issues(issue_id INTEGER PRIMARY KEY,volume TEXT,number TEXT);CREATE TABLE articles(article_id INTEGER PRIMARY KEY,journal_id INTEGER,issue_id INTEGER,title TEXT,publication_year INTEGER,date TEXT,authors_json TEXT,start_page TEXT,end_page TEXT,doi TEXT,pmid TEXT);INSERT INTO journals VALUES(1,'canonical','Journal','["1234-5679"]');INSERT INTO issues VALUES(2,'4','5');INSERT INTO articles VALUES(9007199254740993,1,2,'Article',2026,'2026-05','["Alice","Bob"]','10','20','10.1234/test',NULL)`)
	if err != nil {
		t.Fatal(err)
	}
	return configuration, database
}

// TestArticleLocatorReadsOnlyCanonicalJoinedFields verifies joined identity, optional issues and exact missing-row errors.
func TestArticleLocatorReadsOnlyCanonicalJoinedFields(t *testing.T) {
	configuration, database := locatorDatabase(t)
	locator, err := GetArticleLocator(context.Background(), configuration, nil, identity.Id(9007199254740993))
	if err != nil {
		t.Fatal(err)
	}
	assertCanonicalLocatorIdentity(t, locator)
	assertCanonicalLocatorIssue(t, locator)
	if _, err := database.Exec("UPDATE articles SET issue_id=NULL"); err != nil {
		t.Fatal(err)
	}
	locator, err = GetArticleLocator(context.Background(), configuration, nil, identity.Id(9007199254740993))
	if err != nil || locator.Volume != nil || locator.IssueNumber != nil {
		t.Fatalf("optional issue lost: %#v %v", locator, err)
	}
	_, err = GetArticleLocator(context.Background(), configuration, nil, 1)
	var missing query.NotFound
	if !errors.As(err, &missing) || err.Error() != "Article not found" {
		t.Fatal(err)
	}
}

func TestArticleLocatorRejectsMalformedJsonAndStorageClasses(t *testing.T) {
	for _, statement := range []string{
		`UPDATE journals SET issns_json='null'`,
		`UPDATE journals SET issns_json='[null]'`,
		`UPDATE articles SET authors_json='{"name":"Alice"}'`,
		`UPDATE articles SET authors_json='["Alice",1]'`,
		`UPDATE articles SET publication_year=2026.5`,
		`UPDATE articles SET title=x'6162'`,
	} {
		t.Run(statement, func(t *testing.T) {
			configuration, database := locatorDatabase(t)
			if _, err := database.Exec(statement); err != nil {
				t.Fatal(err)
			}
			if _, err := GetArticleLocator(context.Background(), configuration, nil, identity.Id(9007199254740993)); err == nil {
				t.Fatal("malformed canonical data accepted")
			}
		})
	}
}

// assertCanonicalLocatorIdentity checks the same captured joined bibliographic identity and author fields.
func assertCanonicalLocatorIdentity(t *testing.T, locator domain.ArticleLocator) {
	t.Helper()
	if locator.ArticleId != 9007199254740993 || locator.CatalogId != "canonical" || locator.JournalTitle != "Journal" || locator.Title != "Article" || !reflect.DeepEqual(locator.JournalIssns, []string{"1234-5679"}) || !reflect.DeepEqual(locator.Authors, []string{"Alice", "Bob"}) {
		t.Fatalf("%#v", locator)
	}
}

// assertCanonicalLocatorIssue checks the same optional joined issue and identifier values.
func assertCanonicalLocatorIssue(t *testing.T, locator domain.ArticleLocator) {
	t.Helper()
	if *locator.Volume != "4" || *locator.IssueNumber != "5" || *locator.Doi != "10.1234/test" || locator.Pmid != nil {
		t.Fatalf("%#v", locator)
	}
}
