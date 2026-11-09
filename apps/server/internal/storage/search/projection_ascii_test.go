package search

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// TestProjectionAsciiSeparators preserves every ASCII byte in both unchanged and normalized text.
func TestProjectionAsciiSeparators(t *testing.T) {
	for character := rune(0); character < 128; character++ {
		t.Run(fmt.Sprintf("%02x", character), func(t *testing.T) {
			value := string(character)
			expected := " "
			if character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' {
				expected = value
			}
			if actual := PrepareText("!"+value+"É", true); actual != " "+expected+"E" {
				t.Fatalf("normalized ASCII %02x: got %q", character, actual)
			}
			if strings.ContainsRune(" \t\n\r\f", character) {
				expected = value
			}
			if actual := PrepareText(value, true); actual != expected {
				t.Fatalf("standalone ASCII %02x: got %q, want %q", character, actual, expected)
			}
			if actual := PrepareText("!"+value+"É", false); actual != "!"+value+"É" {
				t.Fatalf("plain projection changed ASCII %02x: %q", character, actual)
			}
		})
	}
}

// BenchmarkPrepareArticleText covers short search/title text and realistic abstract lengths.
func BenchmarkPrepareArticleText(b *testing.B) {
	for _, scenario := range []struct{ name, sentence string }{
		{"English", "Results: the model predicts growth (p < 0.05), with 95% confidence. "},
		{"Accents", "Résumé: Café models predict naïve outcomes; région and année vary. "},
		{"Chinese", "研究结果表明：模型能够解释经济增长，并支持进一步的实证分析。"},
		{"Mixed", "Results: 模型 predicts growth (p < 0.05); 数据支持进一步分析。 "},
		{"AsciiFast", "Research methods and results support the proposed model "},
	} {
		for _, repeats := range []int{1, 20, 80} {
			b.Run(fmt.Sprintf("%s/%d", scenario.name, repeats), func(b *testing.B) {
				value := strings.Repeat(scenario.sentence, repeats)
				b.ReportAllocs()
				b.SetBytes(int64(len(value)))
				for b.Loop() {
					PrepareText(value, true)
				}
			})
		}
	}
}

// BenchmarkRebuildArticleText includes native FTS publication and committed caller transactions.
func BenchmarkRebuildArticleText(b *testing.B) {
	for _, count := range []int{100, 1000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			ctx := context.Background()
			database, err := storage.OpenPlain(filepath.Join(b.TempDir(), "search.sqlite"))
			if err != nil {
				b.Fatal(err)
			}
			defer database.Close()
			connection, err := database.Conn(ctx)
			if err != nil {
				b.Fatal(err)
			}
			defer connection.Close()
			if err := storage.LoadSimple(connection); err != nil {
				b.Fatal(err)
			}
			if _, err := connection.ExecContext(ctx, `CREATE TABLE journals(journal_id INTEGER PRIMARY KEY,title TEXT);
CREATE TABLE articles(article_id INTEGER PRIMARY KEY,title TEXT,abstract_text TEXT,doi TEXT,pmid TEXT,authors_json TEXT,journal_id INTEGER);
INSERT INTO journals VALUES(1,'Research: methods & results');
CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,tokenize='simple 0');`); err != nil {
				b.Fatal(err)
			}
			abstract := strings.Repeat("Results: the model predicts growth (p < 0.05), with 95% confidence. ", 20)
			if _, err := connection.ExecContext(ctx, `WITH RECURSIVE sequence(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM sequence WHERE id<?)
INSERT INTO articles SELECT id,printf('Research: article %d',id),?,printf('10.1234/article.%d',id),NULL,'["A. Smith","B. Jones"]',1 FROM sequence`, count, abstract); err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			for b.Loop() {
				if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
					b.Fatal(err)
				}
				if err := Rebuild(ctx, connection); err != nil {
					b.Fatal(err)
				}
				if _, err := connection.ExecContext(ctx, "COMMIT"); err != nil {
					b.Fatal(err)
				}
			}
			var matches int
			if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM article_search WHERE article_search MATCH 'abstract_text:confidence'").Scan(&matches); err != nil || matches != count {
				b.Fatal("rebuilt search membership changed", matches, err)
			}
			var canonical string
			if err := connection.QueryRowContext(ctx, "SELECT abstract_text FROM articles WHERE article_id=1").Scan(&canonical); err != nil || canonical != abstract {
				b.Fatal("canonical abstract changed", canonical, err)
			}
		})
	}
}
