package index

import (
	"context"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func batchFixture(t *testing.T) (*Connection, IndexBatch, BatchRequest, string) {
	t.Helper()
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "batch.sqlite")
	connection, err := OpenBatch(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	entry, _ := contentInput(t)
	request, err := NewBatchRequest([]CatalogInput{{Filename: "journal.csv", CatalogName: "journal", CsvSha256: strings.Repeat("a", 64), ProviderName: "scholarly", Entries: []domain.JournalCatalogEntry{entry}}}, "all", domain.Incremental, 10, true, false)
	if err != nil {
		t.Fatal(err)
	}
	admission, err := AdmitBatch(ctx, connection.Conn, request, true, "owner", 100)
	if err != nil {
		t.Fatal(err)
	}
	return connection, admission.Batch, request, path
}

func TestBatchV1QuarantinesAmbiguousNotifications(t *testing.T) {
	for _, state := range []struct {
		phase    string
		exit     *int32
		expected NotifyStatus
	}{{"notifying", nil, NotifyUnknown}, {"notifying", ptr(int32(0)), NotifyUnknown}, {"completed", ptr(int32(0)), NotifyCompleted}, {"completed", ptr(int32(1)), NotifyUnknown}, {"indexing", ptr(int32(0)), NotifyUnknown}, {"indexing", nil, ""}} {
		t.Run(fmt.Sprintf("%s-%v", state.phase, state.expected), func(t *testing.T) {
			connection, batch, _, path := batchFixture(t)
			ctx := context.Background()
			if _, err := connection.ExecContext(ctx, `UPDATE index_batch_catalogs SET phase=?1,notify_exit_code=?2`, state.phase, state.exit); err != nil {
				t.Fatal(err)
			}
			for _, column := range []string{"notify_attempt_id", "notify_status", "notify_unknown_acknowledged_attempt_id", "notify_unknown_acknowledged_at"} {
				if _, err := connection.ExecContext(ctx, "ALTER TABLE index_batch_catalogs DROP COLUMN "+column); err != nil {
					t.Fatal(err)
				}
			}
			if _, err := connection.ExecContext(ctx, "PRAGMA user_version=1"); err != nil {
				t.Fatal(err)
			}
			if err := InitBatch(ctx, connection.Conn); err != nil {
				t.Fatal(err)
			}
			other, err := OpenBatch(ctx, path)
			if err != nil {
				t.Fatal(err)
			}
			defer other.Close()
			values, err := ReadBatchCatalogs(ctx, other.Conn, batch.BatchId)
			if err != nil {
				t.Fatal(err)
			}
			handoff := values[0].NotifyHandoff
			if state.expected == "" {
				if handoff != nil {
					t.Fatal(handoff)
				}
				return
			}
			if handoff == nil || handoff.Status != state.expected || !sameOptional(handoff.ExitCode, state.exit) || !strings.HasPrefix(handoff.AttemptId, "legacy-notify-") || len(handoff.AttemptId) != 46 {
				t.Fatalf("handoff=%+v", handoff)
			}
			first := handoff.AttemptId
			if err := InitBatch(ctx, other.Conn); err != nil {
				t.Fatal(err)
			}
			values, err = ReadBatchCatalogs(ctx, other.Conn, batch.BatchId)
			if err != nil || values[0].NotifyHandoff.AttemptId != first {
				t.Fatalf("v2 reopen changed attempt: %v", err)
			}
		})
	}
}

func TestBatchFuturePreflightPreservesBytes(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "future.sqlite")
	database, err := sqlite.OpenMigration(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.ExecContext(ctx, "CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('keep'); PRAGMA user_version=3"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	before, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	connection, err := OpenBatch(ctx, path)
	if connection != nil {
		connection.Close()
		t.Fatal("future database accepted")
	}
	var version *BatchError
	if !errors.As(err, &version) || version.Kind != "unsupported_version" {
		t.Fatal(err)
	}
	after, err := os.ReadFile(path)
	if err != nil || !reflect.DeepEqual(before, after) {
		t.Fatalf("future bytes changed: %v", err)
	}
}

func TestBatchAdmissionAndReplacementRollback(t *testing.T) {
	connection, batch, request, _ := batchFixture(t)
	ctx := context.Background()
	admission, err := AdmitBatch(ctx, connection.Conn, request, false, "owner", 101)
	if err != nil || !admission.IsAbandoning {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, `CREATE TRIGGER reject_catalog BEFORE INSERT ON index_batch_catalogs BEGIN SELECT RAISE(FAIL,'injected catalog failure'); END`); err != nil {
		t.Fatal(err)
	}
	before := selectedSnapshot(t, connection.Conn, []string{"index_batches", "index_batch_catalogs", "index_batch_lease"})
	if _, err := ReplaceAbandoningBatch(ctx, connection.Conn, batch.BatchId, request, "owner", 102); err == nil {
		t.Fatal("replacement succeeded through failure")
	}
	after := selectedSnapshot(t, connection.Conn, []string{"index_batches", "index_batch_catalogs", "index_batch_lease"})
	if !reflect.DeepEqual(before, after) {
		t.Fatal("failed replacement lost the abandoning owner or history")
	}
	if _, err := connection.ExecContext(ctx, "DROP TRIGGER reject_catalog"); err != nil {
		t.Fatal(err)
	}
	replacement, err := ReplaceAbandoningBatch(ctx, connection.Conn, batch.BatchId, request, "owner", 103)
	if err != nil || replacement.BatchId == batch.BatchId {
		t.Fatal(err)
	}
}

func TestBatchCompetingConnectionsAndLeaseBoundary(t *testing.T) {
	connection, batch, request, path := batchFixture(t)
	ctx := context.Background()
	other, err := OpenBatch(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer other.Close()
	if _, err := AdmitBatch(ctx, other.Conn, request, true, "other", 399); err == nil {
		t.Fatal("unexpired lease stolen")
	}
	admission, err := AdmitBatch(ctx, other.Conn, request, true, "other", 400)
	if err != nil || admission.Batch.BatchId != batch.BatchId {
		t.Fatal(err)
	}
	if err := TransitionCatalogPhase(ctx, connection.Conn, batch.BatchId, "owner", 0, CatalogIndexing, 401); err == nil {
		t.Fatal("stale owner wrote")
	}
	if err := HeartbeatBatchLease(ctx, other.Conn, batch.BatchId, "other", 699); err != nil {
		t.Fatal(err)
	}
	if err := HeartbeatBatchLease(ctx, other.Conn, batch.BatchId, "other", 999); err == nil {
		t.Fatal("expired lease revived by heartbeat")
	}
	assertBatchLeaseSaturates(t, connection, other, request)
}

func TestBatchMalformedRecoveryMetadataRejected(t *testing.T) {
	for _, mutation := range []string{
		"journal_count=-1", "phase='unknown'", "run_id='r',written_article_count=-1,source_attempt_count=1", "run_id='r',written_article_count=1,source_attempt_count=-1", "notify_attempt_id='a',notify_status='completed',notify_exit_code=NULL", "notify_attempt_id='a',notify_status='running',notify_exit_code=0", "notify_attempt_id='a',notify_status='unknown',notify_exit_code=2147483648", "notify_attempt_id='a',notify_status='unknown',notify_exit_code=1.5", "manifest_payload='text',manifest_sha256=printf('%064d',0)", "manifest_payload=x'01',manifest_sha256=printf('%064d',0),manifest_path='x',manifest_run_id='r',manifest_generated_at='1'", "notify_attempt_id='a',notify_status='unknown',notify_unknown_acknowledged_attempt_id='b',notify_unknown_acknowledged_at=-1",
	} {
		t.Run(mutation, func(t *testing.T) {
			connection, batch, _, _ := batchFixture(t)
			ctx := context.Background()
			if _, err := connection.ExecContext(ctx, "PRAGMA ignore_check_constraints=ON; UPDATE index_batch_catalogs SET "+mutation); err != nil {
				t.Fatal(err)
			}
			if _, err := ReadBatchCatalogs(ctx, connection.Conn, batch.BatchId); err == nil {
				t.Fatal("malformed state accepted")
			}
		})
	}
}

func TestBatchIntentFailureDoesNotAdvancePhase(t *testing.T) {
	connection, batch, _, _ := batchFixture(t)
	ctx := context.Background()
	if err := TransitionCatalogPhase(ctx, connection.Conn, batch.BatchId, "owner", 0, CatalogIndexing, 100); err != nil {
		t.Fatal(err)
	}
	intent, err := NewManifestIntent([]byte("{}\n"), ptr(int64(9)), "changes.json", "run", "100")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, `CREATE TRIGGER reject_touch BEFORE UPDATE OF updated_at ON index_batches BEGIN SELECT RAISE(FAIL,'injected touch failure'); END`); err != nil {
		t.Fatal(err)
	}
	if err := StoreManifestIntent(ctx, connection.Conn, batch.BatchId, "owner", 0, intent, 101); err == nil {
		t.Fatal("injected failure ignored")
	}
	values, err := ReadBatchCatalogs(ctx, connection.Conn, batch.BatchId)
	if err != nil || values[0].ManifestIntent != nil || values[0].Phase != CatalogIndexing {
		t.Fatalf("partly committed intent: %v %v", values, err)
	}
}

func assertBatchLeaseSaturates(t *testing.T, connection, other *Connection, request BatchRequest) {
	t.Helper()
	ctx := context.Background()
	if batchLeaseExpiry(math.MaxInt64-1) != math.MaxInt64 {
		t.Fatal("expiry overflow")
	}
	if _, err := AdmitBatch(ctx, other.Conn, request, true, "other", math.MaxInt64-1); err != nil {
		t.Fatal(err)
	}
	var expiry int64
	if err := connection.QueryRowContext(ctx, "SELECT expires_at FROM index_batch_lease").Scan(&expiry); err != nil || expiry != math.MaxInt64 {
		t.Fatal(expiry, err)
	}
}
