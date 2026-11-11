package runtime

import (
	"context"
	"database/sql"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/process"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
)

func manualFixture(t *testing.T) (*Prepared, *delivery.RunRecord) {
	t.Helper()
	configuration := runtimeConfiguration(t)
	prepared, err := prepareResources(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := prepared.Close(); err != nil {
			t.Errorf("prepared.Close: %v", err)
		}
	})
	user, err := prepared.services.Auth.Bootstrap(context.Background(), "fixture", "hash", "salt", unixTime(), nil)
	if err != nil {
		t.Fatal(err)
	}
	userID := int64(user.ID)
	deadline := unixTime() + 300
	outcome, err := prepared.services.Delivery.AdmitManualRun(context.Background(), delivery.RunCreate{ExternalID: "fixture-run", Workflow: delivery.WorkflowPush, ScopeKey: "manual:user", TriggerKind: delivery.TriggerKindManual, Mode: delivery.RunModeExecute, UserID: &userID, DeadlineAt: &deadline, CreatedAt: unixTime()})
	if err != nil {
		t.Fatal(err)
	}
	return prepared, outcome.Run
}

func TestManualDispatchSpawnFailureFinalizesOnlyQueuedRun(t *testing.T) {
	prepared, run := manualFixture(t)
	prepared.configuration.Executable = filepath.Join(t.TempDir(), "missing-binary")
	dispatcher := manualDispatcher{configuration: prepared.configuration, repository: prepared.services.Delivery}
	if err := dispatcher.dispatch(2); err != nil {
		t.Fatal(err)
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), run.ID)
	if err != nil || stored.Status != delivery.RunStatusFailed || stored.ErrorCode == nil || *stored.ErrorCode != "spawn_or_assign_failed" || len(dispatcher.children) != 0 {
		t.Fatal("spawn failure lost durable classification", stored, err)
	}
}

// TestManualFinalizationReportsPersistenceFailure prevents failed durable writes from appearing successful.
func TestManualFinalizationReportsPersistenceFailure(t *testing.T) {
	for _, scenario := range []string{"forced owner", "forced queued deadline", "queued deadline", "spawn failure"} {
		t.Run(scenario, func(t *testing.T) {
			prepared, run := manualFixture(t)
			dispatcher := manualDispatcher{configuration: prepared.configuration, repository: prepared.services.Delivery}
			dispatcher.configuration.Executable = filepath.Join(t.TempDir(), "missing-binary")
			active := &manualChild{runID: run.ID, ownerID: "owner", stopKind: "deadline"}
			if scenario == "forced owner" {
				if _, err := dispatcher.repository.ClaimRun(context.Background(), run.ID, active.ownerID, run.Revision, unixTime(), 300); err != nil {
					t.Fatal(err)
				}
			}
			if err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
				if scenario == "queued deadline" {
					if _, err := connection.ExecContext(context.Background(), "UPDATE delivery_runs SET created_at=?,deadline_at=? WHERE id=?", unixTime()-2, unixTime()-1, run.ID); err != nil {
						return err
					}
				}
				_, err := connection.ExecContext(context.Background(), "CREATE TRIGGER reject_terminal_write BEFORE UPDATE OF status ON delivery_runs BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END")
				return err
			}); err != nil {
				t.Fatal(err)
			}
			var err error
			if strings.HasPrefix(scenario, "forced") {
				err = dispatcher.finalizeForced(active)
			} else {
				err = dispatcher.dispatch(1)
			}
			if err == nil {
				t.Fatal("terminal persistence failure was ignored")
			}
			if strings.Contains(err.Error(), "injected") {
				t.Fatal("storage failure detail was exposed", err)
			}
		})
	}
}

// TestExpiredManualCandidatePreservesConcurrentClaim keeps stale queued observations from stopping dispatch.
func TestExpiredManualCandidatePreservesConcurrentClaim(t *testing.T) {
	prepared, run := manualFixture(t)
	dispatcher := manualDispatcher{repository: prepared.services.Delivery}
	if _, err := dispatcher.repository.ClaimRun(context.Background(), run.ID, "replacement", run.Revision, unixTime(), 300); err != nil {
		t.Fatal(err)
	}
	if isExpired, err := dispatcher.expireQueuedManualCandidate(*run, *run.DeadlineAt); err != nil || !isExpired {
		t.Fatal("concurrent claim was treated as a persistence failure", isExpired, err)
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), run.ID)
	if err != nil || stored.OwnerID == nil || *stored.OwnerID != "replacement" || stored.Status != delivery.RunStatusClaimed {
		t.Fatal("stale candidate changed replacement ownership", stored, err)
	}
}

func TestForcedManualFinalizationPreservesReplacementOwnerAndTerminalResult(t *testing.T) {
	prepared, run := manualFixture(t)
	repository := prepared.services.Delivery
	claimed, err := repository.ClaimRun(context.Background(), run.ID, "replacement", run.Revision, unixTime(), 300)
	if err != nil {
		t.Fatal(err)
	}
	dispatcher := manualDispatcher{repository: repository}
	active := &manualChild{runID: run.ID, ownerID: "previous", stopKind: "shutdown"}
	assertReplacementManualOwnerProtected(t, &dispatcher, active, claimed.Run.Revision)
	active.ownerID = "replacement"
	revision := assertOwnedManualRunQuarantined(t, &dispatcher, active)
	active.stopKind = "deadline"
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err := repository.LoadRun(context.Background(), run.ID)
	if err != nil || stored.Revision != revision || *stored.ErrorCode != "forced_shutdown_unknown" {
		t.Fatal("terminal result was overwritten", stored, err)
	}
}

func TestManualRuntimeChild(t *testing.T) {
	filename := os.Getenv("LITRADAR_RUNTIME_CHILD_ADDRESS")
	if filename == "" {
		return
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(7)
	}
	if err := os.WriteFile(filename, []byte(listener.Addr().String()), 0600); err != nil {
		os.Exit(8)
	}
	for {
		connection, err := listener.Accept()
		if err != nil {
			os.Exit(9)
		}
		if err := connection.Close(); err != nil {
			t.Fatalf("connection.Close: %v", err)
		}
	}
}

func runtimeChild(t *testing.T) (*process.Child, string) {
	t.Helper()
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(t.TempDir(), "address")
	environment := []string{}
	for _, value := range os.Environ() {
		if !strings.HasPrefix(value, "LITRADAR_RUNTIME_CHILD_ADDRESS=") {
			environment = append(environment, value)
		}
	}
	environment = append(environment, "LITRADAR_RUNTIME_CHILD_ADDRESS="+filename)
	child, err := process.Start(context.Background(), process.Config{Path: executable, Args: []string{"-test.run=^TestManualRuntimeChild$"}, Environment: environment})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := child.Close(); err != nil {
			t.Errorf("child.Close: %v", err)
		}
	})
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if address, err := os.ReadFile(filename); err == nil {
			return child, string(address)
		}
		if isDone, err := child.Poll(); isDone {
			t.Fatal("child exited before listening", err)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("child listener did not start")
	return nil, ""
}

func TestManualShutdownReapsChildBeforeQuarantiningOwnedRun(t *testing.T) {
	prepared, run := manualFixture(t)
	owner := "manual-owner"
	if _, err := prepared.services.Delivery.ClaimRun(context.Background(), run.ID, owner, run.Revision, unixTime(), 300); err != nil {
		t.Fatal(err)
	}
	child, address := runtimeChild(t)
	dispatcher := manualDispatcher{repository: prepared.services.Delivery, children: []*manualChild{{runID: run.ID, ownerID: owner, child: child}}}
	if err := dispatcher.shutdown(); err != nil {
		t.Fatal(err)
	}
	if connection, err := net.DialTimeout("tcp", address, 100*time.Millisecond); err == nil {
		if err := connection.Close(); err != nil {
			t.Fatalf("connection.Close: %v", err)
		}
		t.Fatal("dispatcher released a live child")
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), run.ID)
	if err != nil || stored.Status != delivery.RunStatusUnknown || stored.ErrorCode == nil || *stored.ErrorCode != "forced_shutdown_unknown" || len(dispatcher.children) != 0 {
		t.Fatal(fmt.Sprint("shutdown state: ", stored, err))
	}
}

// assertReplacementManualOwnerProtected checks stale finalization cannot alter the replacement owner.
func assertReplacementManualOwnerProtected(t *testing.T, dispatcher *manualDispatcher, active *manualChild, claimedRevision int64) {
	t.Helper()
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), active.runID)
	if err != nil || stored.Revision != claimedRevision || stored.OwnerID == nil || *stored.OwnerID != "replacement" {
		t.Fatal("forced termination overwrote replacement ownership", stored, err)
	}
}

// assertOwnedManualRunQuarantined checks owned ambiguous finalization before terminal preservation.
func assertOwnedManualRunQuarantined(t *testing.T, dispatcher *manualDispatcher, active *manualChild) int64 {
	t.Helper()
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), active.runID)
	if err != nil || stored.Status != delivery.RunStatusUnknown || stored.ErrorCode == nil || *stored.ErrorCode != "forced_shutdown_unknown" {
		t.Fatal("owned ambiguous run was not quarantined", stored, err)
	}
	revision := stored.Revision
	return revision
}
