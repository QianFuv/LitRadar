package index

import (
	"context"

	"errors"
	"os"
	"path/filepath"

	"strconv"
	"strings"
	"testing"
)

func TestPublicationFailureRetainsOutboxAndPreparedCursor(t *testing.T) {
	ctx := context.Background()
	directory := t.TempDir()
	content, err := OpenContent(ctx, filepath.Join(directory, "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer content.Close()
	catalog, batch := contentInput(t)
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
