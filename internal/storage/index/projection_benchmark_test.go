package index

import (
	"context"
	"fmt"
	"path/filepath"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"
)

// projectionBenchmarkFixture seeds canonical articles through the real batch transaction.
func projectionBenchmarkFixture(test testing.TB, count, version int) (*Connection, domain.JournalCatalogEntry) {
	test.Helper()
	filename := filepath.Join(test.TempDir(), "content.sqlite")
	if version != indexschema.Version {
		if err := contentfixture.Create(filename, version); err != nil {
			test.Fatal(err)
		}
	}
	connection, err := OpenContent(context.Background(), filename)
	if err != nil {
		test.Fatal(err)
	}
	test.Cleanup(func() { connection.Close() })
	catalog, batch := contentfixture.Batch()
	catalog.Title = "Revue française d’économie & systèmes — 期刊"
	template := batch.Articles[0]
	batch.Articles = make([]domain.ArticleDraft, count)
	for index := range batch.Articles {
		article := template
		doi := fmt.Sprintf("10.1234/projected.%d", index)
		article.Doi = &doi
		article.Title = fmt.Sprintf("Research article %d", index)
		batch.Articles[index] = article
	}
	if outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "initial", "epoch"); err != nil || outcome.ArticlesChanged != uint64(count) {
		test.Fatal("initial projection failed", outcome, err)
	}
	return connection, catalog
}

// assertJournalProjection verifies canonical title, FTS membership, and unchanged durable outbox rows.
func assertJournalProjection(test testing.TB, connection *Connection, catalog domain.JournalCatalogEntry, count int) {
	test.Helper()
	journal := JournalId(catalog.CatalogId)
	var actualTitle string
	if err := connection.QueryRowContext(context.Background(), "SELECT title FROM journals WHERE journal_id=?", journal).Scan(&actualTitle); err != nil || actualTitle != catalog.Title {
		test.Fatal("canonical title changed", actualTitle, err)
	}
	for _, statement := range []string{
		"SELECT count(*) FROM articles WHERE journal_id=?",
		"SELECT count(*) FROM article_listing WHERE journal_id=?",
		"SELECT count(*) FROM article_search WHERE article_search MATCH 'journal_title:francaise' AND rowid IN (SELECT article_id FROM articles WHERE journal_id=?)",
		"SELECT count(*) FROM article_change_events WHERE journal_id=?",
	} {
		var actual int
		if err := connection.QueryRowContext(context.Background(), statement, journal).Scan(&actual); err != nil || actual != count {
			test.Fatal("projection or outbox count changed", statement, actual, count, err)
		}
	}
}

// TestJournalTitleRefreshPreservesCanonicalRows verifies current and legacy tokenizer publications.
func TestJournalTitleRefreshPreservesCanonicalRows(t *testing.T) {
	for _, version := range []int{6, 9, indexschema.Version} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			connection, catalog := projectionBenchmarkFixture(t, 2, version)
			catalog.Title = "Revue française: économie & systèmes — mise à jour"
			catalog.Area = new("Updated area")
			if err := ReconcileCatalogIdentities(context.Background(), connection.Conn, []domain.JournalCatalogEntry{catalog}); err != nil {
				t.Fatal(err)
			}
			assertJournalProjection(t, connection, catalog, 2)
			for query, expected := range map[string]int{"journal_title:mise": 2, "journal_title:期刊": 0} {
				var matches int
				if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM article_search WHERE article_search MATCH ? AND rowid IN (SELECT article_id FROM articles WHERE journal_id=?)", query, JournalId(catalog.CatalogId)).Scan(&matches); err != nil || matches != expected {
					t.Fatal("stored journal terms were not replaced", query, matches, expected, err)
				}
			}
			var updated int
			if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM article_listing WHERE journal_id=? AND area='Updated area'", JournalId(catalog.CatalogId)).Scan(&updated); err != nil || updated != 2 {
				t.Fatal("area refresh changed", updated, err)
			}
			var storedVersion int
			if err := connection.QueryRowContext(context.Background(), "PRAGMA user_version").Scan(&storedVersion); err != nil || storedVersion != version {
				t.Fatal("catalog refresh changed schema version", storedVersion, err)
			}
		})
	}
}

// BenchmarkJournalTitleRefresh measures complete catalog reconciliation with actual FTS publications.
func BenchmarkJournalTitleRefresh(b *testing.B) {
	for _, count := range []int{225, 2000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			connection, catalog := projectionBenchmarkFixture(b, count, indexschema.Version)
			titles := []string{"Revue française: économie & systèmes — édition A", "Revue française: économie & systèmes — édition B"}
			iteration := 0
			b.ReportAllocs()
			for b.Loop() {
				catalog.Title = titles[iteration%len(titles)]
				if err := ReconcileCatalogIdentities(context.Background(), connection.Conn, []domain.JournalCatalogEntry{catalog}); err != nil {
					b.Fatal(err)
				}
				iteration++
			}
			assertJournalProjection(b, connection, catalog, count)
		})
	}
}

// projectionUpdateBatch changes incoming page metadata without triggering richer-text merge stabilization.
func projectionUpdateBatch(count int, startPage string) domain.ProviderBatch {
	_, batch := contentfixture.Batch()
	template := batch.Articles[0]
	batch.Articles = make([]domain.ArticleDraft, count)
	for index := range batch.Articles {
		article := template
		article.Doi = new(fmt.Sprintf("10.1234/projected.%d", index))
		article.Title = fmt.Sprintf("Research article %d", index)
		article.StartPage = &startPage
		batch.Articles[index] = article
	}
	return batch
}

// BenchmarkContentBatchProjection publishes genuine changes while keeping journal title and outbox size bounded.
func BenchmarkContentBatchProjection(b *testing.B) {
	for _, count := range []int{1, 225} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			connection, catalog := projectionBenchmarkFixture(b, count, indexschema.Version)
			batches := []domain.ProviderBatch{projectionUpdateBatch(count, "1"), projectionUpdateBatch(count, "2")}
			iteration := 0
			b.ReportAllocs()
			for b.Loop() {
				outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batches[iteration%2], "updated", "epoch")
				if err != nil || outcome.ArticlesChanged != uint64(count) {
					b.Fatal("benchmark stopped publishing article changes", outcome, err)
				}
				iteration++
			}
		})
	}
}

// TestContentBatchJournalProjectionTracksCatalogChanges checks publication and replay across supported tokenizer layouts.
func TestContentBatchJournalProjectionTracksCatalogChanges(t *testing.T) {
	for _, version := range []int{6, 9, indexschema.Version} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			connection, catalog := projectionBenchmarkFixture(t, 2, version)
			assertJournalProjection(t, connection, catalog, 2)
			catalog.Title = "Revue française — édition suivante"
			if err := ReconcileCatalogIdentities(context.Background(), connection.Conn, []domain.JournalCatalogEntry{catalog}); err != nil {
				t.Fatal(err)
			}
			batch := projectionUpdateBatch(2, "1")
			outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "updated", "epoch")
			if err != nil || outcome.ArticlesChanged != 2 {
				t.Fatal("changed batch did not publish", outcome, err)
			}
			for query, expected := range map[string]int{"journal_title:francaise": 2, "journal_title:suivante": 2, "journal_title:期刊": 0} {
				var actual int
				if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM article_search WHERE article_search MATCH ? AND rowid IN (SELECT article_id FROM articles WHERE journal_id=?)", query, JournalId(catalog.CatalogId)).Scan(&actual); err != nil || actual != expected {
					t.Fatal("batch retained stale journal projection", query, actual, err)
				}
			}
			var title string
			if err := connection.QueryRowContext(context.Background(), "SELECT title FROM journals WHERE journal_id=?", JournalId(catalog.CatalogId)).Scan(&title); err != nil || title != catalog.Title {
				t.Fatal("batch changed canonical title", title, err)
			}
			outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "replay", "epoch")
			if err != nil || outcome.ArticlesChanged != 0 || outcome.ChangeEventsEmitted != 0 {
				t.Fatal("unchanged batch replay published changes", outcome, err)
			}
		})
	}
}
