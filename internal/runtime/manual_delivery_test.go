package runtime

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/process"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
)

func manualFixture(t *testing.T) (*Prepared, *delivery.RunRecord) {
	t.Helper()
	configuration := runtimeConfiguration(t)
	prepared, err := prepareResources(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { prepared.Close() })
	user, err := prepared.services.Auth.Bootstrap(context.Background(), "fixture", "hash", "salt", unixTime(), nil)
	if err != nil {
		t.Fatal(err)
	}
	userId := int64(user.Id)
	deadline := unixTime() + 300
	outcome, err := prepared.services.Delivery.AdmitManualRun(context.Background(), delivery.RunCreate{ExternalId: "fixture-run", Workflow: delivery.WorkflowPush, ScopeKey: "manual:user", TriggerKind: delivery.TriggerKindManual, Mode: delivery.RunModeExecute, UserId: &userId, DeadlineAt: &deadline, CreatedAt: unixTime()})
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
	stored, err := dispatcher.repository.LoadRun(context.Background(), run.Id)
	if err != nil || stored.Status != delivery.RunStatusFailed || stored.ErrorCode == nil || *stored.ErrorCode != "spawn_or_assign_failed" || len(dispatcher.children) != 0 {
		t.Fatal("spawn failure lost durable classification", stored, err)
	}
}

func TestForcedManualFinalizationPreservesReplacementOwnerAndTerminalResult(t *testing.T) {
	prepared, run := manualFixture(t)
	repository := prepared.services.Delivery
	claimed, err := repository.ClaimRun(context.Background(), run.Id, "replacement", run.Revision, unixTime(), 300)
	if err != nil {
		t.Fatal(err)
	}
	dispatcher := manualDispatcher{repository: repository}
	active := &manualChild{runId: run.Id, ownerId: "previous", stopKind: "shutdown"}
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err := repository.LoadRun(context.Background(), run.Id)
	if err != nil || stored.Revision != claimed.Run.Revision || stored.OwnerId == nil || *stored.OwnerId != "replacement" {
		t.Fatal("forced termination overwrote replacement ownership", stored, err)
	}
	active.ownerId = "replacement"
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err = repository.LoadRun(context.Background(), run.Id)
	if err != nil || stored.Status != delivery.RunStatusUnknown || stored.ErrorCode == nil || *stored.ErrorCode != "forced_shutdown_unknown" {
		t.Fatal("owned ambiguous run was not quarantined", stored, err)
	}
	revision := stored.Revision
	active.stopKind = "deadline"
	if err := dispatcher.finalizeForced(active); err != nil {
		t.Fatal(err)
	}
	stored, err = repository.LoadRun(context.Background(), run.Id)
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
		connection.Close()
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
	t.Cleanup(func() { child.Close() })
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
	if _, err := prepared.services.Delivery.ClaimRun(context.Background(), run.Id, owner, run.Revision, unixTime(), 300); err != nil {
		t.Fatal(err)
	}
	child, address := runtimeChild(t)
	dispatcher := manualDispatcher{repository: prepared.services.Delivery, children: []*manualChild{{runId: run.Id, ownerId: owner, child: child}}}
	if err := dispatcher.shutdown(); err != nil {
		t.Fatal(err)
	}
	if connection, err := net.DialTimeout("tcp", address, 100*time.Millisecond); err == nil {
		connection.Close()
		t.Fatal("dispatcher released a live child")
	}
	stored, err := dispatcher.repository.LoadRun(context.Background(), run.Id)
	if err != nil || stored.Status != delivery.RunStatusUnknown || stored.ErrorCode == nil || *stored.ErrorCode != "forced_shutdown_unknown" || len(dispatcher.children) != 0 {
		t.Fatal(fmt.Sprint("shutdown state: ", stored, err))
	}
}
