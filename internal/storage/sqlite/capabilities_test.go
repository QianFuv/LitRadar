package sqlite_test

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/index"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	native "github.com/mattn/go-sqlite3"
)

// compatibilityConnection pins independently initialized old and candidate native connections.
func compatibilityConnection(t *testing.T, filename string, hasStatic bool) (*sql.DB, *sql.Conn) {
	t.Helper()
	instance := &native.SQLiteDriver{}
	if hasStatic {
		instance.ConnectHook = (*native.SQLiteConn).RegisterSimple
	} else {
		oracleRoot, err := filepath.Abs("../../../target/simple-tokenizer-oracle")
		if err != nil {
			t.Fatal(err)
		}
		body, err := os.ReadFile(filepath.Join(oracleRoot, "inputs.json"))
		if err != nil {
			t.Fatal("required old tokenizer oracle unavailable", err)
		}
		var identity struct {
			Filename, Sha256 string
			Target           struct{ Goos, Goarch string }
		}
		if err := json.Unmarshal(body, &identity); err != nil {
			t.Fatal(err)
		}
		library := filepath.Join(oracleRoot, identity.Filename)
		body, err = os.ReadFile(library)
		if err != nil {
			t.Fatal(err)
		}
		hash := sha256.Sum256(body)
		if hex.EncodeToString(hash[:]) != identity.Sha256 || identity.Target.Goos != runtime.GOOS || identity.Target.Goarch != runtime.GOARCH {
			t.Fatal("old oracle bytes or target identity changed")
		}
		t.Logf("legacy oracle target=%s/%s sha256=%s", runtime.GOOS, runtime.GOARCH, identity.Sha256)
		instance.ConnectHook = func(connection *native.SQLiteConn) error {
			return connection.LoadExtension(library, "sqlite3_simple_init")
		}
	}
	name := "simple-compatibility-" + t.TempDir()
	sql.Register(name, instance)
	database, err := sql.Open(name, filename)
	if err != nil {
		t.Fatal(err)
	}
	database.SetMaxOpenConns(1)
	connection, err := database.Conn(context.Background())
	if err != nil {
		database.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close(); database.Close() })
	return database, connection
}

// compatibilityRows records all native values in a stable order, including NULL contentless columns.
func compatibilityRows(t *testing.T, connection *sql.Conn, query string, arguments ...any) [][]any {
	t.Helper()
	rows, err := connection.QueryContext(context.Background(), query, arguments...)
	if err != nil {
		t.Fatal(query, arguments, err)
	}
	defer rows.Close()
	columns, err := rows.Columns()
	if err != nil {
		t.Fatal(err)
	}
	result := [][]any{}
	for rows.Next() {
		values := make([]any, len(columns))
		pointers := make([]any, len(columns))
		for position := range values {
			pointers[position] = &values[position]
		}
		if err := rows.Scan(pointers...); err != nil {
			t.Fatal(err)
		}
		for position, value := range values {
			if bytes, ok := value.([]byte); ok {
				values[position] = string(bytes)
			}
		}
		result = append(result, values)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return result
}

// compatibilityExec fails immediately when an oracle fixture cannot represent the production contract.
func compatibilityExec(t *testing.T, connection *sql.Conn, statement string, arguments ...any) {
	t.Helper()
	if _, err := connection.ExecContext(context.Background(), statement, arguments...); err != nil {
		t.Fatal(statement, err)
	}
}

// TestStaticSimpleCompatibility compares text offsets and a closed legacy v9 index without rebuilding it.
func TestStaticSimpleCompatibility(t *testing.T) {
	corpus := [][2]string{
		{"科技金融如何赋能企业", "中文期刊 technology"},
		{"科技 新 金融", "金融科技"},
		{"科技科技金融", "AB12cd 00123 alpha ALPHA"},
		{"Résumé École cafe\u0301 CAFÉ", "中文，English-test@example.com"},
		{"𠀀😀中文", "\t金融\n科技 \"quoted\""},
		{"", "科技金融 abstract only"},
	}
	queries := []string{`"科技金融"`, `科技 AND 金融`, `alpha*`, `"AB12cd"`, `NEAR("科技" "金融", 0)`, `title:"科技"`, `科技 OR alpha`, `科技 NOT 金融`, `zhongwen`, `zw`}
	for _, hasProjection := range []bool{false, true} {
		t.Run(map[bool]string{false: "raw", true: "production-projection"}[hasProjection], func(t *testing.T) {
			_, previous := compatibilityConnection(t, ":memory:", false)
			_, candidate := compatibilityConnection(t, ":memory:", true)
			for _, connection := range []*sql.Conn{previous, candidate} {
				compatibilityExec(t, connection, "CREATE VIRTUAL TABLE corpus USING fts5(title,abstract_text,tokenize='simple 0'); CREATE VIRTUAL TABLE vocabulary USING fts5vocab(corpus,instance)")
				for position, document := range corpus {
					compatibilityExec(t, connection, "INSERT INTO corpus(rowid,title,abstract_text) VALUES(?,?,?)", position+1, search.PrepareText(document[0], hasProjection), search.PrepareText(document[1], hasProjection))
				}
			}
			vocabulary := "SELECT term,doc,col,offset FROM vocabulary ORDER BY doc,col,offset,term"
			if old, current := compatibilityRows(t, previous, vocabulary), compatibilityRows(t, candidate, vocabulary); !reflect.DeepEqual(old, current) {
				t.Fatalf("token positions changed\nold=%v\nstatic=%v", old, current)
			}
			for _, query := range queries {
				statement := "SELECT rowid,highlight(corpus,0,'[',']'),snippet(corpus,1,'[',']','...',16),simple_highlight(corpus,0,'[',']'),simple_snippet(corpus,1,'[',']','...',16),simple_highlight_pos(corpus,0) FROM corpus WHERE corpus MATCH ? ORDER BY rowid"
				if old, current := compatibilityRows(t, previous, statement, query), compatibilityRows(t, candidate, statement, query); !reflect.DeepEqual(old, current) {
					t.Fatalf("query/highlight changed for %q\nold=%v\nstatic=%v", query, old, current)
				}
				if (query == "zhongwen" || query == "zw") && len(compatibilityRows(t, candidate, statement, query)) != 0 {
					t.Fatal("simple 0 enabled pinyin aliases")
				}
			}
		})
	}
	t.Run("existing-v9", func(t *testing.T) {
		filename := filepath.Join(t.TempDir(), "previous-v9.sqlite")
		previousDatabase, previous := compatibilityConnection(t, filename, false)
		// seed creates independent full v9 schemas from the same canonical corpus.
		seed := func(connection *sql.Conn) {
			compatibilityExec(t, connection, indexschema.ContentTables)
			compatibilityExec(t, connection, "INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json) VALUES(1,'compat','期刊','[]','[]')")
			for position, document := range corpus {
				identifier := position + 1
				compatibilityExec(t, connection, "INSERT INTO articles(article_id,journal_id,title,abstract_text,authors_json) VALUES(?,1,?,?,'[]')", identifier, document[0], document[1])
				compatibilityExec(t, connection, "INSERT INTO article_listing(article_id,journal_id) VALUES(?,1)", identifier)
				compatibilityExec(t, connection, "INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(?,?,?,?,'','','',?)", identifier, identifier, search.PrepareText(document[0], true), search.PrepareText(document[1], true), search.PrepareText("期刊", true))
			}
			compatibilityExec(t, connection, "PRAGMA user_version=9; CREATE VIRTUAL TABLE temp.vocabulary USING fts5vocab(main,article_search,instance)")
		}
		seed(previous)
		_, control := compatibilityConnection(t, ":memory:", true)
		seed(control)
		vocabulary := "SELECT term,doc,col,offset FROM vocabulary ORDER BY doc,col,offset,term"
		oldVocabulary := compatibilityRows(t, previous, vocabulary)
		oldRecords := compatibilityRows(t, previous, "SELECT article_id,title,abstract_text FROM articles ORDER BY article_id")
		oldSchema := compatibilityRows(t, previous, "SELECT name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
		oldMatches := compatibilityRows(t, previous, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", `"科技金融"`)
		previous.Close()
		previousDatabase.Close()
		candidate, err := index.OpenContent(context.Background(), filename)
		if err != nil {
			t.Fatal(err)
		}
		defer candidate.Close()
		compatibilityExec(t, candidate.Conn, "CREATE VIRTUAL TABLE temp.vocabulary USING fts5vocab(main,article_search,instance)")
		for _, pair := range []struct {
			query string
			old   [][]any
		}{
			{vocabulary, oldVocabulary},
			{"SELECT article_id,title,abstract_text FROM articles ORDER BY article_id", oldRecords},
			{"SELECT name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name", oldSchema},
		} {
			if current := compatibilityRows(t, candidate.Conn, pair.query); !reflect.DeepEqual(pair.old, current) {
				t.Fatalf("legacy v9 changed on reopen: %s", pair.query)
			}
		}
		if current := compatibilityRows(t, candidate.Conn, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", `"科技金融"`); !reflect.DeepEqual(oldMatches, current) {
			t.Fatal("legacy MATCH results changed")
		}
		for _, connection := range []*sql.Conn{candidate.Conn, control} {
			compatibilityExec(t, connection, "INSERT INTO articles(article_id,journal_id,title,authors_json) VALUES(99,1,'科技金融新增','[]'); INSERT INTO article_listing(article_id,journal_id) VALUES(99,1)")
			compatibilityExec(t, connection, "INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(99,99,'科技金融新增','','','','','期刊')")
			compatibilityExec(t, connection, "UPDATE articles SET title='科技金融更新',abstract_text='' WHERE article_id=2; UPDATE article_search SET article_id=2,title='科技金融更新',abstract_text='',doi='',pmid='',authors='',journal_title='期刊' WHERE rowid=2; DELETE FROM article_search WHERE rowid=3; DELETE FROM article_listing WHERE article_id=3; DELETE FROM articles WHERE article_id=3")
		}
		for _, statement := range []string{vocabulary, "SELECT article_id,title,abstract_text FROM articles ORDER BY article_id"} {
			if current, expected := compatibilityRows(t, candidate.Conn, statement), compatibilityRows(t, control, statement); !reflect.DeepEqual(current, expected) {
				t.Fatalf("legacy v9 differs from fresh static control: %s", statement)
			}
		}
		compatibilityExec(t, candidate.Conn, "INSERT INTO article_search(article_search) VALUES('integrity-check')")
		if rows := compatibilityRows(t, candidate.Conn, "PRAGMA integrity_check"); !reflect.DeepEqual(rows, [][]any{{"ok"}}) {
			t.Fatal(rows)
		}
		if rows := compatibilityRows(t, candidate.Conn, "PRAGMA foreign_key_check"); len(rows) != 0 {
			t.Fatal(rows)
		}
		if rows := compatibilityRows(t, candidate.Conn, "PRAGMA user_version"); !reflect.DeepEqual(rows, [][]any{{int64(9)}}) {
			t.Fatal("v9 version changed", rows)
		}
		newVocabulary := compatibilityRows(t, candidate.Conn, vocabulary)
		newMatches := compatibilityRows(t, candidate.Conn, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", `"科技金融"`)
		candidate.Close()
		_, reopened := compatibilityConnection(t, filename, false)
		compatibilityExec(t, reopened, "CREATE VIRTUAL TABLE temp.vocabulary USING fts5vocab(main,article_search,instance)")
		if current := compatibilityRows(t, reopened, vocabulary); !reflect.DeepEqual(current, newVocabulary) {
			t.Fatal("old tokenizer cannot read candidate positions")
		}
		if current := compatibilityRows(t, reopened, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", `"科技金融"`); !reflect.DeepEqual(current, newMatches) {
			t.Fatal("old tokenizer cannot search candidate append/update")
		}
	})
}
