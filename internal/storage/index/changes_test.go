package index

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestOriginalRustManifestBytes(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/index/manifest-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name, Database, Run, Generated string
				Events                         []struct {
					EventId   string  `json:"event_id"`
					ArticleId string  `json:"article_id"`
					JournalId string  `json:"journal_id"`
					IssueId   *string `json:"issue_id"`
					InPress   string  `json:"in_press"`
					Kind      string
				}
			}
			Expected struct {
				Payload string
				Through *string
				Count   uint64
				Limits  []map[string]any
			}
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, item := range corpus.Observations {
		t.Run(item.Input.Name, func(t *testing.T) {
			ctx := context.Background()
			database, err := sqlite.OpenMigration(filepath.Join(t.TempDir(), "events.sqlite"))
			if err != nil {
				t.Fatal(err)
			}
			defer database.Close()
			connection, err := database.Conn(ctx)
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			if _, err := connection.ExecContext(ctx, "CREATE TABLE article_change_events(event_id INTEGER PRIMARY KEY,content_revision TEXT,article_id INTEGER,change_kind TEXT,journal_id INTEGER,issue_id INTEGER,in_press INTEGER,created_at TEXT)"); err != nil {
				t.Fatal(err)
			}
			if err := immediate(ctx, connection, func() error {
				for _, event := range item.Input.Events {
					parse := func(value string) int64 {
						number, err := strconv.ParseInt(value, 10, 64)
						if err != nil {
							t.Fatal(err)
						}
						return number
					}
					var issue *int64
					if event.IssueId != nil {
						issue = ptr(parse(*event.IssueId))
					}
					if _, err := connection.ExecContext(ctx, "INSERT INTO article_change_events VALUES(?1,'revision',?2,?3,?4,?5,?6,'epoch')", parse(event.EventId), parse(event.ArticleId), event.Kind, parse(event.JournalId), issue, parse(event.InPress)); err != nil {
						return err
					}
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
			prepared, err := PrepareContentChangeManifest(ctx, connection, item.Input.Database, item.Input.Run, item.Input.Generated)
			if err != nil {
				t.Fatal(err)
			}
			var through *string
			if prepared.ThroughEventId != nil {
				through = ptr(strconv.FormatInt(*prepared.ThroughEventId, 10))
			}
			if string(prepared.Payload) != item.Expected.Payload || !reflect.DeepEqual(through, item.Expected.Through) || prepared.EventCount != item.Expected.Count {
				t.Fatalf("actual=%s through=%v count=%d\nexpected=%+v", prepared.Payload, through, prepared.EventCount, item.Expected)
			}
			for position, limit := range []uint64{0, 1, 1000, 10000, 10001} {
				events, err := ListContentChangeEvents(ctx, connection, 0, limit)
				actual := map[string]any{"count": float64(len(events))}
				if err != nil {
					actual = map[string]any{"error": err.Error()}
				}
				if !reflect.DeepEqual(actual, item.Expected.Limits[position]) {
					t.Fatalf("limit=%d actual=%v expected=%v", limit, actual, item.Expected.Limits[position])
				}
			}
		})
	}
}

func TestPublicationFailureRetainsOutboxAndPreparedCursor(t *testing.T) {
	ctx := context.Background()
	directory := t.TempDir()
	content, err := OpenContent(ctx, filepath.Join(directory, "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer content.Close()
	catalog, batch := frozenContentInput(t)
	if _, err := WriteContentBatch(ctx, content.Conn, catalog, batch, "first", "epoch"); err != nil {
		t.Fatal(err)
	}
	blocked := filepath.Join(directory, "blocked")
	if err := os.Mkdir(blocked, 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := WriteContentChangeManifest(ctx, content.Conn, "example.sqlite", "run", "100", blocked); err == nil {
		t.Fatal("published over directory")
	}
	events, err := ListContentChangeEvents(ctx, content.Conn, 0, 100)
	if err != nil || len(events) != 1 {
		t.Fatalf("lost outbox after failure: %v %v", events, err)
	}
	if _, err := os.Stat(manifestTempPath(blocked)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("temporary leak: %v", err)
	}
	prepared, err := PrepareContentChangeManifest(ctx, content.Conn, "example.sqlite", "run", "100")
	if err != nil {
		t.Fatal(err)
	}
	batch.Articles[0].AbstractText = ptr("Additional enrichment")
	if _, err := WriteContentBatch(ctx, content.Conn, catalog, batch, "second", "epoch"); err != nil {
		t.Fatal(err)
	}
	current := filepath.Join(directory, "current.json")
	history := filepath.Join(directory, "history", "immutable.json")
	if err := PublishContentChangeHistory(history, prepared.Payload); err != nil {
		t.Fatal(err)
	}
	if err := PublishContentChangeManifest(current, prepared.Payload); err != nil {
		t.Fatal(err)
	}
	count, err := AcknowledgeContentChangeEvents(ctx, content.Conn, *prepared.ThroughEventId)
	if err != nil || count != 1 {
		t.Fatalf("ack=%d %v", count, err)
	}
	events, err = ListContentChangeEvents(ctx, content.Conn, 0, 100)
	if err != nil || len(events) != 1 || events[0].ContentRevision != "second" {
		t.Fatalf("inclusive cursor lost late event: %v %v", events, err)
	}
	if err := os.WriteFile(current, []byte("damaged"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := PublishContentChangeManifest(current, prepared.Payload); err != nil {
		t.Fatal(err)
	}
	if err := PublishContentChangeHistory(history, prepared.Payload); err != nil {
		t.Fatal(err)
	}
	if err := PublishContentChangeHistory(history, []byte("different")); !errors.Is(err, os.ErrExist) {
		t.Fatalf("history overwritten: %v", err)
	}
	for _, path := range []string{current, history} {
		body, err := os.ReadFile(path)
		if err != nil || string(body) != string(prepared.Payload) {
			t.Fatalf("saved publication changed: %s %v", path, err)
		}
	}
}

func TestHistoryPruningUsesOnlyCanonicalEpochAndManagedName(t *testing.T) {
	directory := t.TempDir()
	values := []string{`"99"`, `"100"`, `"101"`, `" 99 "`, `"+99"`, `"099"`, `"-0"`, `99`, `null`, `"2026-01-01T00:00:00Z"`, `"-1"`}
	for position, value := range values {
		path := filepath.Join(directory, strings.Repeat("0", 63)+strconv.FormatInt(int64(position), 16)+".changes.json")
		if err := os.WriteFile(path, []byte(`{"generated_at":`+value+`}`), 0600); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{"other.json", strings.Repeat("A", 64) + ".changes.json"} {
		if err := os.WriteFile(filepath.Join(directory, name), []byte("invalid JSON"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	count, err := PruneContentChangeHistory(directory, 100)
	if err != nil || count != 3 {
		t.Fatalf("pruned=%d err=%v", count, err)
	}
	for position := range values {
		path := filepath.Join(directory, strings.Repeat("0", 63)+strconv.FormatInt(int64(position), 16)+".changes.json")
		_, err := os.Stat(path)
		shouldRemain := position != 0 && position != 3 && position != 10
		if shouldRemain != (err == nil) {
			t.Fatalf("position=%d err=%v", position, err)
		}
	}
	bad := filepath.Join(directory, strings.Repeat("f", 64)+".changes.json")
	if err := os.WriteFile(bad, []byte("invalid JSON"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := PruneContentChangeHistory(directory, 100); err == nil {
		t.Fatal("malformed managed history silently skipped")
	}
	if count, err := PruneContentChangeHistory(filepath.Join(directory, "missing"), 100); err != nil || count != 0 {
		t.Fatalf("missing=%d %v", count, err)
	}
}
