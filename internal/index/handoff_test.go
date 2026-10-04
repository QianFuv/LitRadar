package index

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func indexOracle(t *testing.T, input map[string]any) map[string]any {
	t.Helper()
	if runtime.GOOS != "windows" {
		t.Skip("live Rust file handoff runs on Windows; both platforms execute frozen Rust workflows")
	}
	body, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	binary, err := filepath.Abs("../../output/migration/execution/index-oracle/identity.exe")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(binary); os.IsNotExist(err) {
		t.Skip("live Rust handoff requires the separately preserved migration oracle; the migration runner requires this test to pass")
	} else if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, binary)
	command.Stdin = bytes.NewReader(append(body, '\n'))
	var output, diagnostics bytes.Buffer
	command.Stdout, command.Stderr = &output, &diagnostics
	if err := command.Run(); err != nil {
		t.Fatalf("Rust observer failed: %v %s", err, diagnostics.Bytes())
	}
	var observed struct{ Expected map[string]any }
	decoder := json.NewDecoder(&output)
	decoder.UseNumber()
	if err := decoder.Decode(&observed); err != nil {
		t.Fatal(err)
	}
	if value, exists := observed.Expected["error"]; exists {
		t.Fatalf("Rust observer rejected workflow: %v", value)
	}
	for _, value := range observed.Expected["operations"].([]any) {
		if result, ok := value.(map[string]any); ok {
			if failure, exists := result["error"]; exists {
				t.Fatalf("Rust operation failed: %v", failure)
			}
		}
	}
	return observed.Expected
}

func TestOriginalRustGoContentFileHandoff(t *testing.T) {
	for _, version := range []int{0, 6, 7, 8, 9} {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			if runtime.GOOS != "windows" {
				t.Skip("native Rust file handoff is required by the Windows proof runner")
			}
			config := liveFixture(t, "alpha")
			input, err := FreezeCatalog(filepath.Join(config.ProjectRoot, "data", "meta", "alpha.csv"), "cnki")
			if err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(config.ProjectRoot, "handoff.sqlite")
			if version > 0 {
				body, err := os.ReadFile("../../tests/migration/storage/fixtures/content-v" + strconv.Itoa(version) + ".sqlite.fixture")
				if err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(path, body, 0600); err != nil {
					t.Fatal(err)
				}
			}
			entry, batch := input.Entries[0], liveBatch(input.Entries[0])
			write := func(revision string) map[string]any {
				return map[string]any{"op": "write", "catalog": entry, "batch": batch, "revision": revision}
			}
			indexOracle(t, map[string]any{"op": "content", "path": path, "operations": []any{write("rust-first")}})
			connection, err := storage.OpenContent(context.Background(), path)
			if err != nil {
				t.Fatal(err)
			}
			batch.Articles[0].Title = "Durable article with a longer Go title"
			outcome, err := storage.WriteContentBatch(context.Background(), connection.Conn, entry, batch, "go-second", "epoch")
			connection.Close()
			if err != nil || outcome.ArticlesChanged != 1 {
				t.Fatalf("Rust to Go=%+v err=%v", outcome, err)
			}
			batch.Articles[0].Title += " and a final original Rust update"
			observed := indexOracle(t, map[string]any{"op": "content", "path": path, "operations": []any{write("rust-third")}})
			if observed["operations"].([]any)[0].(map[string]any)["articles_changed"] != json.Number("1") {
				t.Fatal("Rust did not update Go-written content")
			}
			connection, err = storage.OpenContent(context.Background(), path)
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			var count int
			if err := connection.Conn.QueryRowContext(context.Background(), "SELECT count(*) FROM articles WHERE doi=?1 AND title=?2", *batch.Articles[0].Doi, batch.Articles[0].Title).Scan(&count); err != nil || count != 1 {
				t.Fatalf("stable identity after handoff count=%d err=%v", count, err)
			}
		})
	}
}

func TestOriginalRustGoControlFileHandoff(t *testing.T) {
	path := filepath.Join(t.TempDir(), "control.sqlite")
	indexOracle(t, map[string]any{"op": "control", "path": path, "operations": []any{
		map[string]any{"op": "prepare"}, map[string]any{"op": "advance", "checkpoint": "opaque-rust-checkpoint"},
	}})
	ctx := context.Background()
	connection, err := storage.OpenControl(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	run := storage.SyncRun{Scope: storage.SyncScope{CatalogName: "catalog", ProviderName: "scholarly", CatalogId: "catalog:example"}, BatchId: "batch-1", RunId: "go-run", Mode: domain.Incremental}
	preparation, err := storage.PrepareJournalSync(ctx, connection.Conn, run, true, "go-epoch")
	if err != nil || preparation.Checkpoint == nil || preparation.Checkpoint.TraversalCheckpoint == nil || *preparation.Checkpoint.TraversalCheckpoint != "opaque-rust-checkpoint" {
		t.Fatalf("Rust to Go checkpoint=%+v err=%v", preparation, err)
	}
	if err := storage.CompleteSyncRun(ctx, connection.Conn, run, testCheckpoint("opaque-go-anchor"), "go-epoch"); err != nil {
		t.Fatal(err)
	}
	connection.Close()
	observed := indexOracle(t, map[string]any{"op": "control", "path": path, "operations": []any{map[string]any{"op": "anchor"}, map[string]any{"op": "prepare", "batch": "batch-2", "run": "rust-again"}}})
	if observed["operations"].([]any)[0].(map[string]any)["committed_anchor"] != "opaque-go-anchor" {
		t.Fatal("Rust lost Go committed anchor")
	}
	connection, err = storage.OpenControl(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	checkpoint, err := storage.ReadRunCheckpoint(ctx, connection.Conn, run.Scope)
	if err != nil || checkpoint == nil || checkpoint.BaseAnchor == nil || *checkpoint.BaseAnchor != "opaque-go-anchor" || checkpoint.RunId != "rust-again" {
		t.Fatalf("Rust restored checkpoint=%+v err=%v", checkpoint, err)
	}
}

func TestOriginalRustGoBatchFileHandoff(t *testing.T) {
	path := filepath.Join(t.TempDir(), "batch.sqlite")
	config := liveFixture(t, "alpha")
	input, err := FreezeCatalog(filepath.Join(config.ProjectRoot, "data", "meta", "alpha.csv"), "cnki")
	if err != nil {
		t.Fatal(err)
	}
	input.CsvSha256 = strings.Repeat("a", 64)
	request := map[string]any{"catalogs": []any{map[string]any{"filename": input.Filename, "name": input.CatalogName, "digest": input.CsvSha256, "provider": input.ProviderName, "entries": input.Entries}}, "selection": "all", "mode": "incremental", "size": 8, "notify": true, "dry": false}
	outcome := map[string]any{"run": "rust-run", "journals": 1, "written": 1, "attempts": 1, "path": "data/manifest.json"}
	indexOracle(t, map[string]any{"op": "batch", "path": path, "operations": []any{
		map[string]any{"op": "admit", "request": request}, map[string]any{"op": "phase", "phase": "indexing"}, map[string]any{"op": "outcome", "value": outcome},
		map[string]any{"op": "intent", "value": map[string]any{"payload": []int{123, 125, 10}, "through": 1, "path": "data/manifest.json", "run": "rust-run", "generated": "100"}},
		map[string]any{"op": "phase", "phase": "manifest_published"}, map[string]any{"op": "phase", "phase": "notifying"}, map[string]any{"op": "notify_prepare", "attempt": "rust-ambiguous"}, map[string]any{"op": "notify_record", "attempt": "rust-ambiguous", "status": "unknown", "exit": nil}, map[string]any{"op": "release"},
	}})
	ctx := context.Background()
	connection, err := storage.OpenBatch(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	requested, err := storage.NewBatchRequest([]storage.CatalogInput{input}, "all", domain.Incremental, 8, true, false)
	if err != nil {
		t.Fatal(err)
	}
	admission, err := storage.AdmitBatch(ctx, connection.Conn, requested, true, "go-owner", 101)
	if err != nil {
		t.Fatal(err)
	}
	batch := admission.Batch
	blocked, err := storage.PrepareNotifyAttempt(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, "go-attempt", false, 101)
	if err != nil || blocked.Decision != "blocked_unknown" {
		t.Fatalf("Rust ambiguity lost: %+v %v", blocked, err)
	}
	if _, err := storage.PrepareNotifyAttempt(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, "go-attempt", true, 102); err != nil {
		t.Fatal(err)
	}
	zero := int32(0)
	if _, err := storage.RecordNotifyAttemptResult(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, "go-attempt", storage.NotifyCompleted, &zero, 103); err != nil {
		t.Fatal(err)
	}
	if err := storage.ReleaseBatchLease(ctx, connection.Conn, batch.BatchId, batch.OwnerId); err != nil {
		t.Fatal(err)
	}
	connection.Close()
	observed := indexOracle(t, map[string]any{"op": "batch", "path": path, "operations": []any{map[string]any{"op": "admit", "request": request, "now": 104}, map[string]any{"op": "notify_prepare", "attempt": "unused", "now": 104}, map[string]any{"op": "complete_catalog", "value": outcome, "now": 104}, map[string]any{"op": "complete_batch", "now": 104}}})
	state := observed["operations"].([]any)[1].(map[string]any)
	if state["decision"] != "succeeded" || state["state"].(map[string]any)["ack_attempt"] != "rust-ambiguous" {
		t.Fatalf("Go to Rust notification fence=%v", state)
	}
	connection, err = storage.OpenBatch(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	var status string
	if err := connection.Conn.QueryRowContext(ctx, "SELECT status FROM index_batches WHERE batch_id=?1", batch.BatchId).Scan(&status); err != nil || status != "completed" {
		t.Fatalf("final status=%s err=%v", status, err)
	}
}
