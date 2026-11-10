package search

import (
	"fmt"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/indexschema"
)

// TestRebuildStoresEveryFieldAsTokenPositions exercises the contentless ASCII declaration.
func TestRebuildStoresEveryFieldAsTokenPositions(t *testing.T) {
	connection, ctx := searchFixture(t)
	if _, err := connection.ExecContext(ctx, "DROP TABLE article_search;"+indexschema.SearchTable+"CREATE VIRTUAL TABLE temp.vocabulary USING fts5vocab(main,article_search,instance)"); err != nil {
		t.Fatal(err)
	}
	title := `深度学习 Deep-Learning for DOI 10.1000/ABC_9 «测试» Ünïcode "q"`
	punctuation := `!"#$%&'()*+,-./:;<=>?@[\]^_` + "`" + `{|}~`
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET title=?,abstract_text=?,doi=?,pmid=? WHERE article_id=1", title, punctuation, "10.1000/ABC_9", "PM123"); err != nil {
		t.Fatal(err)
	}
	if err := Rebuild(ctx, connection); err != nil {
		t.Fatal(err)
	}
	fields := map[string]struct{ value, expected string }{
		"title":         {title, "深 度 学 习 deep learning for doi 10 1000 abc 9 测 试 unicode q"},
		"abstract_text": {punctuation, ""},
		"doi":           {"10.1000/ABC_9", "10 1000 abc 9"},
		"pmid":          {"PM123", "pm 123"},
		"authors":       {"Émile", "emile"},
		"journal_title": {"JÖURNAL", "journal"},
	}
	for column, field := range fields {
		rows, err := connection.QueryContext(ctx, "SELECT term,offset FROM vocabulary WHERE doc=1 AND col=? ORDER BY offset", column)
		if err != nil {
			t.Fatal(err)
		}
		actual := []string{}
		for rows.Next() {
			var term string
			var offset int
			if err := rows.Scan(&term, &offset); err != nil {
				t.Fatal(err)
			}
			if offset != len(actual) {
				t.Fatalf("%s offset=%d expected=%d", column, offset, len(actual))
			}
			actual = append(actual, term)
		}
		if err := rows.Err(); err != nil {
			t.Fatal(err)
		}
		rows.Close()
		if strings.Join(actual, " ") != field.expected {
			t.Fatalf("%s tokens=%q expected=%q", column, actual, field.expected)
		}
		var count int
		expectedCount := 1
		if field.expected == "" {
			expectedCount = 0
		}
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM article_search WHERE rowid=1 AND article_search MATCH ?", column+":"+Phrase(field.value)).Scan(&count); err != nil || count != expectedCount {
			t.Fatalf("%s phrase hits=%d err=%v", column, count, err)
		}
	}
	var count int
	if err := connection.QueryRowContext(ctx, `SELECT count(*) FROM article_search WHERE article_search MATCH '"深度学习"'`).Scan(&count); err != nil || count != 0 {
		t.Fatalf("unprepared phrase hits=%d err=%v", count, err)
	}
}

// TestEnsureRollsBackObsoleteSearchOnInvalidAuthors protects atomic index replacement.
func TestEnsureRollsBackObsoleteSearchOnInvalidAuthors(t *testing.T) {
	connection, ctx := searchFixture(t)
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET authors_json='null' WHERE article_id=2;BEGIN IMMEDIATE"); err != nil {
		t.Fatal(err)
	}
	if err := Ensure(ctx, connection); err == nil {
		t.Fatal("invalid authors accepted")
	}
	if _, err := connection.ExecContext(ctx, "ROLLBACK"); err != nil {
		t.Fatal(err)
	}
	var title string
	if err := connection.QueryRowContext(ctx, "SELECT title FROM article_search WHERE rowid=99").Scan(&title); err != nil || title != "stale" {
		t.Fatalf("old index lost: %q %v", title, err)
	}
}

// TestPrepareQueryMatchesPrefixAndBooleanPhrases exercises the public grammar against FTS5.
func TestPrepareQueryMatchesPrefixAndBooleanPhrases(t *testing.T) {
	connection, ctx := searchFixture(t)
	if _, err := connection.ExecContext(ctx, "DROP TABLE article_search;"+indexschema.SearchTable); err != nil {
		t.Fatal(err)
	}
	if err := Rebuild(ctx, connection); err != nil {
		t.Fatal(err)
	}
	for expression, expected := range map[string]string{`"RÉSUMÉ 中文!"`: "1", `title:rés*`: "1", `"中文" OR "second"`: "1,2", `"中文" AND "second"`: "", `NEAR("中" "文", 0)`: "1", `authors:emile`: "1", `journal_title:JÖURNAL`: "1,2"} {
		rows, err := connection.QueryContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", PrepareQuery(expression, domain.SearchAdvanced))
		if err != nil {
			t.Fatal(err)
		}
		ids := []string{}
		for rows.Next() {
			var id int
			if err := rows.Scan(&id); err != nil {
				t.Fatal(err)
			}
			ids = append(ids, fmt.Sprint(id))
		}
		if err := rows.Err(); err != nil {
			t.Fatal(err)
		}
		rows.Close()
		if actual := strings.Join(ids, ","); actual != expected {
			t.Fatalf("%s: %s expected %s", expression, actual, expected)
		}
	}
}
