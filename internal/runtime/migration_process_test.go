package runtime

import (
	"context"
	"database/sql"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
)

func TestMain(tests *testing.M) {
	directory := os.Getenv("LITRADAR_RUNTIME_PROCESS_FIXTURE")
	if directory == "" {
		os.Exit(tests.Run())
	}
	marker, err := os.ReadFile(filepath.Join(directory, "fixture.marker"))
	if !isRuntimeProcessFixture(marker, err) {
		os.Exit(90)
	}
	identity := ""
	for ordinal, value := range os.Args {
		if value == scheduler.ParentRunIdArgument && ordinal+1 < len(os.Args) {
			identity = os.Args[ordinal+1]
		}
	}
	if !regexp.MustCompile(`^[a-zA-Z0-9-]+$`).MatchString(identity) {
		os.Exit(91)
	}
	executeRuntimeProcessFixture(directory, identity)
}

func runtimeProcessFixture(t *testing.T, prepared *Prepared) string {
	t.Helper()
	directory := t.TempDir()
	if err := os.WriteFile(filepath.Join(directory, "fixture.marker"), []byte("runtime-process-fixture-v1"), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("LITRADAR_RUNTIME_PROCESS_FIXTURE", directory)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	prepared.configuration.Executable = executable
	return directory
}

func awaitRuntimeChild(t *testing.T, directory, identity string) string {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if body, err := os.ReadFile(filepath.Join(directory, identity+".ready")); err == nil {
			return string(body)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("runtime child did not become ready", identity)
	return ""
}

func requireRuntimeChildStopped(t *testing.T, address string) {
	t.Helper()
	if connection, err := net.DialTimeout("tcp", address, 100*time.Millisecond); err == nil {
		connection.Close()
		t.Fatal("runtime returned with live child", address)
	}
}

func TestManualDispatcherAppliesGlobalTwoChildLimitAcrossUsers(t *testing.T) {
	prepared, first := manualFixture(t)
	directory := runtimeProcessFixture(t, prepared)
	runs := admitManualRuntimeUsers(t, prepared, first)
	dispatcher := manualDispatcher{configuration: prepared.configuration, repository: prepared.services.Delivery}
	defer dispatcher.shutdown()
	if err := dispatcher.dispatch(2); err != nil {
		t.Fatal(err)
	}
	if len(dispatcher.children) != 2 {
		t.Fatal("wrong active limit", len(dispatcher.children))
	}
	var addresses []string
	for _, child := range dispatcher.children {
		addresses = append(addresses, awaitRuntimeChild(t, directory, fmt.Sprintf("manual-delivery-%d", child.runId)))
	}
	if err := dispatcher.dispatch(2); err != nil || len(dispatcher.children) != 2 {
		t.Fatal("global bound exceeded", err)
	}
	assertThirdManualChildStillQueued(t, &dispatcher, directory, runs[2].Id)
	if err := dispatcher.shutdown(); err != nil {
		t.Fatal(err)
	}
	for _, address := range addresses {
		requireRuntimeChildStopped(t, address)
	}
}

func TestSchedulerLoopScansWithActiveClaimsAndDrainsOnFailureOrCancellation(t *testing.T) {
	for _, shouldFail := range []bool{false, true} {
		t.Run(fmt.Sprint(shouldFail), func(t *testing.T) {
			prepared, err := prepareResources(context.Background(), runtimeConfiguration(t))
			if err != nil {
				t.Fatal(err)
			}
			defer prepared.Close()
			prepared.configuration.SchedulerIntervalSeconds = 1
			directory := runtimeProcessFixture(t, prepared)
			sqlExecute := func(statement string, args ...any) {
				t.Helper()
				if err := prepared.services.Auth.Immediate(context.Background(), false, func(connection *sql.Conn) error {
					_, err := connection.ExecContext(context.Background(), statement, args...)
					return err
				}); err != nil {
					t.Fatal(err)
				}
			}
			enqueue := func(name string) int64 {
				t.Helper()
				task, err := prepared.services.Scheduler.Create(context.Background(), domain.Create{Name: name, Job: domain.Job{Kind: "index"}, Cron: "0 0 1 1 *", Timezone: "UTC", TimeoutSeconds: 60, Coalesce: true, Enabled: true}, nil, nil)
				if err != nil {
					t.Fatal(err)
				}
				sqlExecute("UPDATE scheduled_tasks SET created_at=? WHERE id=?", unixTime()+86400, task.Id)
				if _, err := prepared.services.Scheduler.Enqueue(context.Background(), *task, []int64{time.Now().Unix() - 1}); err != nil {
					t.Fatal(err)
				}
				var id int64
				if err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
					return connection.QueryRowContext(context.Background(), "SELECT id FROM scheduled_task_runs WHERE task_id=?", task.Id).Scan(&id)
				}); err != nil {
					t.Fatal(err)
				}
				return id
			}
			first := enqueue("first")
			ctx, cancel := context.WithCancel(context.Background())
			finished := make(chan error, 1)
			go func() { finished <- prepared.runScheduler(ctx) }()
			isFinished := false
			defer func() {
				cancel()
				if !isFinished {
					select {
					case <-finished:
					case <-time.After(10 * time.Second):
						t.Error("scheduler cleanup timed out")
					}
				}
			}()
			addresses := []string{awaitRuntimeChild(t, directory, strconv.FormatInt(first, 10))}
			count := 4
			if shouldFail {
				count = 1
			}
			var later []int64
			for ordinal := range count {
				later = append(later, enqueue(fmt.Sprintf("later-%d", ordinal)))
			}
			for ordinal, id := range later {
				if ordinal == 3 {
					break
				}
				addresses = append(addresses, awaitRuntimeChild(t, directory, strconv.FormatInt(id, 10)))
			}
			for _, address := range addresses {
				connection, err := net.DialTimeout("tcp", address, 100*time.Millisecond)
				if err != nil {
					t.Fatal("previous claim did not remain active", err)
				}
				connection.Close()
			}
			if shouldFail {
				sqlExecute(fmt.Sprintf("CREATE TRIGGER fixture_finish_failure BEFORE UPDATE OF status ON scheduled_task_runs WHEN NEW.id=%d AND NEW.status='success' BEGIN SELECT RAISE(ABORT,'private-fixture-sentinel'); END", first))
				if err := os.WriteFile(filepath.Join(directory, strconv.FormatInt(first, 10)+".release"), nil, 0600); err != nil {
					t.Fatal(err)
				}
			} else {
				if _, err := os.Stat(filepath.Join(directory, strconv.FormatInt(later[3], 10)+".ready")); !os.IsNotExist(err) {
					t.Fatal("fifth claim exceeded capacity", err)
				}
				cancel()
			}
			select {
			case err := <-finished:
				isFinished = true
				if (err != nil) != shouldFail {
					t.Fatal("scheduler result", err)
				}
			case <-time.After(10 * time.Second):
				t.Fatal("scheduler did not drain")
			}
			for _, address := range addresses {
				requireRuntimeChildStopped(t, address)
			}
			var cancelled int
			if err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
				return connection.QueryRowContext(context.Background(), "SELECT count(*) FROM scheduled_task_runs WHERE status='cancelled' AND claim_expires_at IS NULL").Scan(&cancelled)
			}); err != nil {
				t.Fatal(err)
			}
			expected := 4
			if shouldFail {
				expected = 1
			}
			if cancelled != expected {
				t.Fatal("cancelled claims", cancelled, expected)
			}
		})
	}
}

// admitManualRuntimeUsers creates the original three user-scoped queued runs before dispatch.
func admitManualRuntimeUsers(t *testing.T, prepared *Prepared, first *delivery.RunRecord) []*delivery.RunRecord {
	t.Helper()
	runs := []*delivery.RunRecord{first}
	for ordinal := 2; ordinal <= 3; ordinal++ {
		if err := prepared.services.Auth.Immediate(context.Background(), false, func(connection *sql.Conn) error {
			_, err := connection.ExecContext(context.Background(), "INSERT INTO users(id,username,password_hash,salt,created_at,updated_at,token_generation) VALUES(?,?, 'hash','salt',1,1,0)", ordinal, fmt.Sprintf("user%d", ordinal))
			return err
		}); err != nil {
			t.Fatal(err)
		}
		user := int64(ordinal)
		deadline := unixTime() + 300
		outcome, err := prepared.services.Delivery.AdmitManualRun(context.Background(), delivery.RunCreate{ExternalId: fmt.Sprintf("manual-%d", ordinal), Workflow: delivery.WorkflowPush, ScopeKey: fmt.Sprintf("manual:user:%d", ordinal), TriggerKind: delivery.TriggerKindManual, Mode: delivery.RunModeExecute, UserId: &user, DeadlineAt: &deadline, CreatedAt: unixTime()})
		if err != nil {
			t.Fatal(err)
		}
		runs = append(runs, outcome.Run)
	}
	return runs
}

// isRuntimeProcessFixture retains marker and argument admission before identity or socket creation.
func isRuntimeProcessFixture(marker []byte, err error) bool {
	return !(err != nil || string(marker) != "runtime-process-fixture-v1" || len(os.Args) < 2 || (os.Args[1] != "index" && os.Args[1] != "delivery-run"))
}

// executeRuntimeProcessFixture retains exit codes, atomic readiness and release-controlled listener lifetime.
func executeRuntimeProcessFixture(directory, identity string) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(92)
	}
	ready := filepath.Join(directory, identity+".ready")
	if err := os.WriteFile(ready+".tmp", []byte(listener.Addr().String()), 0600); err != nil {
		os.Exit(93)
	}
	if err := os.Rename(ready+".tmp", ready); err != nil {
		os.Exit(94)
	}
	for {
		if _, err := os.Stat(filepath.Join(directory, identity+".release")); err == nil {
			listener.Close()
			os.Exit(0)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// assertThirdManualChildStillQueued checks durable admission and absent readiness before any shutdown.
func assertThirdManualChildStillQueued(t *testing.T, dispatcher *manualDispatcher, directory string, thirdId int64) {
	t.Helper()
	third, err := dispatcher.repository.LoadRun(context.Background(), thirdId)
	if err != nil || third.Status != delivery.RunStatusQueued {
		t.Fatal(third, err)
	}
	if _, err := os.Stat(filepath.Join(directory, fmt.Sprintf("manual-delivery-%d.ready", third.Id))); !os.IsNotExist(err) {
		t.Fatal("third child was spawned", err)
	}
}
