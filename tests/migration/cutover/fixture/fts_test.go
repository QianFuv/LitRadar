package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
	"github.com/QianFuv/LitRadar/internal/storage/search"
)

func TestContentlessSearchMutationAndCanonicalProjection(t *testing.T) {
	var corpus struct {
		Observations []struct {
			Input struct {
				Name       string
				Operations []struct {
					Catalog domain.JournalCatalogEntry
					Batch   domain.ProviderBatch
				}
			}
		}
	}
	body, err := os.ReadFile("../../index/content-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, version := range []int{6, 7, 8, 9} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			ctx := context.Background()
			fixture, err := os.ReadFile(fmt.Sprintf("../../storage/fixtures/content-v%d.sqlite.fixture", version))
			if err != nil {
				t.Fatal(err)
			}
			filename := filepath.Join(t.TempDir(), "content.sqlite")
			if err := os.WriteFile(filename, fixture, 0600); err != nil {
				t.Fatal(err)
			}
			connection, err := storage.OpenContent(ctx, filename)
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			for _, direction := range []string{"ASC", "DESC"} {
				for _, filtered := range []bool{false, true} {
					where, expectedIndex := "", "idx_article_listing_date_id"
					arguments := []any{21, 0}
					if filtered {
						where, expectedIndex = "WHERE l.journal_id=?", "idx_article_listing_journal_date_id"
						arguments = []any{1, 21, 0}
					}
					plans, err := connection.QueryContext(ctx, "EXPLAIN QUERY PLAN SELECT l.article_id,l.date FROM article_listing l "+where+" ORDER BY l.date "+direction+",l.article_id "+direction+" LIMIT ? OFFSET ?", arguments...)
					if err != nil {
						t.Fatal(err)
					}
					var details []string
					for plans.Next() {
						var id, parent, unused int
						var detail string
						if err := plans.Scan(&id, &parent, &unused, &detail); err != nil {
							t.Fatal(err)
						}
						details = append(details, detail)
					}
					if err := plans.Err(); err != nil {
						t.Fatal(err)
					}
					plans.Close()
					plan := strings.Join(details, "\n")
					if !strings.Contains(plan, expectedIndex) || strings.Contains(plan, "USE TEMP B-TREE FOR ORDER BY") {
						t.Fatal(plan)
					}
				}
			}
			var catalog domain.JournalCatalogEntry
			var batch domain.ProviderBatch
			for _, item := range corpus.Observations {
				if item.Input.Name == "first-and-replay" {
					catalog, batch = item.Input.Operations[0].Catalog, item.Input.Operations[0].Batch
				}
			}
			catalog.CatalogId, batch.CatalogId, batch.Journal.CatalogId = "projection", "projection", "projection"
			catalog.Issn, catalog.Eissn, catalog.AllIssns = nil, nil, []string{}
			catalog.Title = "Journalorigin"
			article := &batch.Articles[0]
			article.CatalogId, article.Title = "projection", "Alphaorigin 中文原文"
			abstract, doi, pmid := "Abstractorigin", "10.1000/projection", "7654321"
			article.AbstractText, article.Doi, article.Pmid = &abstract, &doi, &pmid
			write := func(revision string) error {
				_, err := storage.WriteContentBatch(ctx, connection.Conn, catalog, batch, revision, "2026-10-05T00:00:00Z")
				return err
			}
			if err := write("projection:1"); err != nil {
				t.Fatal(err)
			}
			var articleId int64
			if err := connection.QueryRowContext(ctx, "SELECT article_id FROM articles WHERE doi=?", doi).Scan(&articleId); err != nil {
				t.Fatal(err)
			}
			match := func(query string, expected int) {
				t.Helper()
				var count int
				if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM article_search WHERE article_search MATCH ? AND rowid=?", query, articleId).Scan(&count); err != nil || count != expected {
					t.Fatalf("query=%s count=%d expected=%d err=%v", query, count, expected, err)
				}
			}
			for _, query := range []string{"title:alphaorigin", "abstract_text:abstractorigin", "doi:projection", "pmid:7654321", "authors:alice", "journal_title:journalorigin"} {
				match(query, 1)
			}
			article.Title, catalog.Title, abstract = "Betaoriginextended 中文新原文 additional", "Journalupdated", "Abstractupdated"
			if err := write("projection:2"); err != nil {
				t.Fatal(err)
			}
			for _, query := range []string{"title:alphaorigin", "abstract_text:abstractorigin", "journal_title:journalorigin"} {
				match(query, 0)
			}
			for _, query := range []string{"title:betaoriginextended", "abstract_text:abstractupdated", "journal_title:journalupdated", "doi:projection", "pmid:7654321", "authors:alice"} {
				match(query, 1)
			}
			var canonicalTitle string
			if err := connection.QueryRowContext(ctx, "SELECT title FROM articles WHERE article_id=?", articleId).Scan(&canonicalTitle); err != nil || canonicalTitle != "Betaoriginextended 中文新原文 additional" {
				t.Fatal(canonicalTitle, err)
			}
			if _, err := connection.ExecContext(ctx, "CREATE TRIGGER fail_outbox BEFORE INSERT ON article_change_events BEGIN SELECT RAISE(ABORT,'synthetic rollback'); END"); err != nil {
				t.Fatal(err)
			}
			article.Title = "Rollbackmarker much longer title that must replace the current title"
			if err := write("projection:failure"); err == nil {
				t.Fatal("outbox failure did not roll back content")
			}
			match("title:rollbackmarker", 0)
			match("title:betaoriginextended", 1)
			if _, err := connection.ExecContext(ctx, "DROP TRIGGER fail_outbox; BEGIN IMMEDIATE"); err != nil {
				t.Fatal(err)
			}
			if _, err := connection.ExecContext(ctx, "DELETE FROM article_search WHERE rowid=?", articleId); err != nil {
				t.Fatal(err)
			}
			match("title:betaoriginextended", 0)
			if _, err := connection.ExecContext(ctx, "ROLLBACK"); err != nil {
				t.Fatal(err)
			}
			match("title:betaoriginextended", 1)
			before := searchVocabulary(t, ctx, connection.Conn)
			if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
				t.Fatal(err)
			}
			if err := search.Rebuild(ctx, connection.Conn); err != nil {
				t.Fatal(err)
			}
			if _, err := connection.ExecContext(ctx, "COMMIT"); err != nil {
				t.Fatal(err)
			}
			if after := searchVocabulary(t, ctx, connection.Conn); !reflect.DeepEqual(after, before) {
				t.Fatalf("incremental projection differs from canonical rebuild\nbefore=%v\nafter=%v", before, after)
			}
		})
	}
}

func searchVocabulary(t *testing.T, ctx context.Context, connection *sql.Conn) []string {
	t.Helper()
	if _, err := connection.ExecContext(ctx, "CREATE VIRTUAL TABLE IF NOT EXISTS temp.projection_terms USING fts5vocab(main,article_search,instance)"); err != nil {
		t.Fatal(err)
	}
	rows, err := connection.QueryContext(ctx, "SELECT term,doc,col,offset FROM temp.projection_terms ORDER BY term,doc,col,offset")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	result := []string{}
	for rows.Next() {
		var term, column string
		var document, offset int64
		if err := rows.Scan(&term, &document, &column, &offset); err != nil {
			t.Fatal(err)
		}
		result = append(result, fmt.Sprintf("%s|%d|%s|%d", term, document, column, offset))
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return result
}
