package index

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"os/exec"
	"reflect"
	"strconv"
	"testing"
	"time"
)

func TestOriginalRustReadsGoUpgradeAndGoReadsRustWrite(t *testing.T) {
	oracle := os.Getenv("LITRADAR_RUST_STORAGE_DATABASE_ORACLE")
	if oracle == "" {
		t.Skip("explicit interoperability runner supplies the original Rust oracle")
	}
	for _, fixture := range fixtures(t) {
		t.Run(strconv.Itoa(fixture.Version), func(t *testing.T) {
			filename, _ := copyFixture(t, fixture)
			ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
			defer cancel()
			if _, err := Migrate(ctx, filename); err != nil {
				t.Fatal(err)
			}
			counts := map[string]int64{}
			inspectConnection(t, filename, func(connection *sql.Conn) {
				for _, table := range []string{"journals", "journal_identity_keys", "issues", "articles", "article_retraction_dois", "article_identity_keys", "article_listing", "article_change_events", "article_search"} {
					var count int64
					if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM "+table).Scan(&count); err != nil {
						t.Fatal(err)
					}
					counts[table] = count
				}
			})
			request, err := json.Marshal(map[string]any{"operation": "index", "path": filename, "write": true})
			if err != nil {
				t.Fatal(err)
			}
			command := exec.CommandContext(ctx, oracle)
			command.Stdin = bytes.NewReader(append(request, '\n'))
			var diagnostics bytes.Buffer
			command.Stderr = &diagnostics
			output, err := command.Output()
			if err != nil {
				t.Fatalf("%v %s", err, diagnostics.String())
			}
			var result struct {
				Error  *string
				Output struct {
					Counts map[string]int64
					Search []int64
				}
			}
			if err := json.Unmarshal(output, &result); err != nil {
				t.Fatal(err)
			}
			if result.Error != nil {
				t.Fatal(*result.Error)
			}
			if !reflect.DeepEqual(counts, result.Output.Counts) || !reflect.DeepEqual(result.Output.Search, []int64{100}) {
				t.Fatalf("%s", output)
			}
			inspectConnection(t, filename, func(connection *sql.Conn) {
				var title string
				if err := connection.QueryRowContext(ctx, "SELECT title FROM articles WHERE article_id=9999").Scan(&title); err != nil || title != "Rust exchange article" {
					t.Fatalf("%q %v", title, err)
				}
				var id int64
				if err := connection.QueryRowContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH 'exchange'").Scan(&id); err != nil || id != 9999 {
					t.Fatalf("%d %v", id, err)
				}
			})
		})
	}
}
