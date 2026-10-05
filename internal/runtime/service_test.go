package runtime

import (
	"context"
	"database/sql"
	"errors"
	"io"
	"math"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/executor"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/backup"
	"github.com/QianFuv/LitRadar/internal/storage/maintenance"
	sqlite3 "github.com/mattn/go-sqlite3"
)

func TestPrepareRejectsDevelopmentBeforeStorageMutation(t *testing.T) {
	configuration := runtimeConfiguration(t)
	configuration.Host = "0.0.0.0"
	if prepared, err := Prepare(context.Background(), configuration); err == nil || prepared != nil {
		t.Fatal("invalid development server prepared", err)
	}
	if _, err := os.Stat(configuration.Storage.AuthDbPath); !os.IsNotExist(err) {
		t.Fatal("development validation touched storage", err)
	}
}

type panickingListener struct{ net.Listener }

func TestHeartbeatPersistsAndStopsWhilePublicWorkersAreOccupied(t *testing.T) {
	prepared, err := Prepare(context.Background(), runtimeConfiguration(t))
	if err != nil {
		t.Fatal(err)
	}
	defer prepared.Close()
	release, started, workers := make(chan struct{}), make(chan struct{}, 8), make(chan error, 8)
	for range 8 {
		go func() {
			_, err := executor.Run(context.Background(), prepared.services.StoragePool, func() (struct{}, error) {
				started <- struct{}{}
				<-release
				return struct{}{}, nil
			})
			workers <- err
		}()
	}
	for range 8 {
		<-started
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- prepared.runHeartbeat(ctx, "saturated", time.Millisecond) }()
	isJoined := false
	defer func() {
		cancel()
		close(release)
		for range 8 {
			if err := <-workers; err != nil {
				t.Error(err)
			}
		}
		if !isJoined {
			select {
			case <-done:
			case <-time.After(time.Second):
				t.Error("heartbeat cleanup did not join")
			}
		}
	}()
	hasHeartbeat := false
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
			return connection.QueryRowContext(context.Background(), "SELECT EXISTS(SELECT 1 FROM service_heartbeats WHERE instance_id='saturated')").Scan(&hasHeartbeat)
		})
		if err != nil {
			t.Fatal(err)
		}
		if hasHeartbeat {
			break
		}
		time.Sleep(time.Millisecond)
	}
	cancel()
	select {
	case err := <-done:
		isJoined = true
		if err != nil {
			t.Error(err)
		}
	case <-time.After(time.Second):
		t.Error("heartbeat failed to join while public workers were occupied")
	}
	if !hasHeartbeat {
		t.Error("heartbeat waited behind public storage admission")
	}
}

func TestHeartbeatOperationBudgetWithNativeWriteLock(t *testing.T) {
	prepared, err := Prepare(context.Background(), runtimeConfiguration(t))
	if err != nil {
		t.Fatal(err)
	}
	defer prepared.Close()
	database, err := platform.Open(platform.Config{Filename: prepared.configuration.Storage.AuthDbPath, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("BEGIN IMMEDIATE"); err != nil {
		t.Fatal(err)
	}
	defer database.Exec("ROLLBACK")
	type result struct {
		err     error
		elapsed time.Duration
	}
	done := make(chan result, 1)
	started := time.Now()
	go func() {
		err := prepared.recordHeartbeat(context.Background(), "locked")
		done <- result{err, time.Since(started)}
	}()
	time.Sleep(6 * time.Second)
	if _, err := database.Exec("ROLLBACK"); err != nil {
		t.Fatal(err)
	}
	select {
	case outcome := <-done:
		t.Logf("operation budget=5s, controlled lock release=%s, completion=%s", time.Since(started), outcome.elapsed)
		if !errors.Is(outcome.err, context.DeadlineExceeded) {
			t.Fatal("heartbeat operation lost its context deadline", outcome.err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("heartbeat did not join after controlled lock release")
	}
}

func TestHeartbeatCancellationInterruptsExecutingSqlAndJoins(t *testing.T) {
	database, err := platform.Open(platform.Config{Filename: filepath.Join(t.TempDir(), "heartbeat.sqlite"), Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	started := make(chan struct{})
	var once sync.Once
	if err := connection.Raw(func(raw any) error {
		return raw.(*sqlite3.SQLiteConn).RegisterFunc("heartbeat_started", func() int { once.Do(func() { close(started) }); return 0 }, false)
	}); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- heartbeatLoop(ctx, time.Millisecond, func(ctx context.Context) error {
			var value int64
			return connection.QueryRowContext(ctx, `WITH RECURSIVE numbers(value) AS (SELECT heartbeat_started() UNION ALL SELECT value+1 FROM numbers WHERE value<1000000000) SELECT sum(value) FROM numbers`).Scan(&value)
		})
	}()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("SQL did not start")
	}
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal("normal SQL cancellation reported persistence failure", err)
		}
	case <-time.After(time.Second):
		t.Fatal("SQL worker did not join")
	}
	var value int
	if err := connection.QueryRowContext(context.Background(), "SELECT 1").Scan(&value); err != nil || value != 1 {
		t.Fatal("canceled SQL polluted next operation", err)
	}
}

func TestHeartbeatCancellationDoesNotHidePersistenceFailure(t *testing.T) {
	for _, cause := range []error{errors.New("disk failure"), errors.Join(context.Canceled, errors.New("disk failure")), context.DeadlineExceeded} {
		ctx, cancel := context.WithCancel(context.Background())
		err := heartbeatLoop(ctx, time.Millisecond, func(context.Context) error { cancel(); return cause })
		cancel()
		if err == nil || err.Error() != "API heartbeat persistence failed" {
			t.Fatal("persistence failure hidden", err)
		}
	}
}

func (listener panickingListener) Accept() (net.Conn, error) {
	panic("private listener panic payload")
}

func TestHttpChildPanicStillJoinsAndReleasesHeartbeat(t *testing.T) {
	configuration := runtimeConfiguration(t)
	prepared, err := Prepare(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	prepared.listener = panickingListener{prepared.listener}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := prepared.Run(ctx); err == nil || !strings.Contains(err.Error(), "API server task failed") || strings.Contains(err.Error(), "private") {
		t.Fatal("nested panic escaped supervised shutdown", err)
	}
	repository, err := auth.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	if err := repository.WithConnection(ctx, func(connection *sql.Conn) error {
		var count int
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM service_heartbeats WHERE service='api'").Scan(&count); err != nil {
			return err
		}
		if count != 0 {
			t.Error("panic retained API heartbeat", count)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestPrepareChecksRecoveryBeforeMigrationAndSecurityBeforeBinding(t *testing.T) {
	configuration := runtimeConfiguration(t)
	paths, err := maintenance.Paths(configuration.Storage)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(paths.Staging, 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := Prepare(context.Background(), configuration); err == nil {
		t.Fatal("interrupted maintenance admitted")
	}
	if _, err := os.Stat(configuration.Storage.AuthDbPath); !os.IsNotExist(err) {
		t.Fatal("migration ran before recovery guard", err)
	}
	configuration = runtimeConfiguration(t)
	configuration.IsDevelopment = false
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	configuration.Port = uint16(listener.Addr().(*net.TCPAddr).Port)
	if _, err := Prepare(context.Background(), configuration); err == nil || strings.Contains(err.Error(), "bind") {
		t.Fatal("listener checked before deployed frontend security", err)
	}
	if _, err := os.Stat(configuration.Storage.AuthDbPath); err != nil {
		t.Fatal("ordered migrations did not precede frontend validation", err)
	}
}

func TestCoordinatorCancelsAndJoinsAfterUnexpectedReturn(t *testing.T) {
	for _, shouldPanic := range []bool{false, true} {
		t.Run(map[bool]string{false: "return", true: "panic"}[shouldPanic], func(t *testing.T) {
			started, cancelled, release := make(chan struct{}), make(chan struct{}), make(chan struct{})
			done := make(chan error, 1)
			go func() {
				done <- coordinate(context.Background(), []component{{"failed", func(context.Context) error {
					<-started
					if shouldPanic {
						panic("private detail must not escape")
					}
					return nil
				}}, {"draining", func(ctx context.Context) error {
					close(started)
					<-ctx.Done()
					close(cancelled)
					<-release
					return nil
				}}})
			}()
			select {
			case <-cancelled:
			case <-time.After(5 * time.Second):
				t.Fatal("failure did not cancel peers")
			}
			select {
			case err := <-done:
				t.Fatal("coordinator abandoned draining work", err)
			default:
			}
			close(release)
			if err := <-done; err == nil || strings.Contains(err.Error(), "private detail") {
				t.Fatal("unexpected return or panic not classified safely", err)
			}
		})
	}
}

func TestCloseWaitsForCancelledCallerWorkerBeforeClosingRepository(t *testing.T) {
	prepared, err := Prepare(context.Background(), runtimeConfiguration(t))
	if err != nil {
		t.Fatal(err)
	}
	defer prepared.Close()
	if err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "CREATE TABLE owned_write(value INTEGER)")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	started, release := make(chan struct{}), make(chan struct{})
	finished := make(chan error, 1)
	caller := make(chan error, 1)
	go func() {
		_, err := executor.Run(ctx, prepared.services.StoragePool, func() (struct{}, error) {
			close(started)
			<-release
			err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
				_, err := connection.ExecContext(context.Background(), "INSERT INTO owned_write VALUES(42)")
				return err
			})
			finished <- err
			return struct{}{}, err
		})
		caller <- err
	}()
	<-started
	cancel()
	if err := <-caller; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	closed := make(chan error, 1)
	go func() { closed <- prepared.Close() }()
	select {
	case err := <-closed:
		t.Fatal("resources released while worker still owns them", err)
	case <-time.After(25 * time.Millisecond):
	}
	close(release)
	if err := <-finished; err != nil {
		t.Fatal("worker lost borrowed repository", err)
	}
	if err := <-closed; err != nil {
		t.Fatal(err)
	}
	repository, err := auth.Open(prepared.configuration.Storage.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	if err := repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		var value int
		if err := connection.QueryRowContext(context.Background(), "SELECT value FROM owned_write").Scan(&value); err != nil {
			return err
		}
		if value != 42 {
			t.Error("admitted write did not commit")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestCoordinatorPreservesCleanupFailuresDuringCancellation(t *testing.T) {
	for range 30 {
		ctx, cancel := context.WithCancel(context.Background())
		cancel()
		failure := errors.New("cleanup did not complete")
		err := coordinate(ctx, []component{{"cleanup", func(context.Context) error { return errors.Join(context.Canceled, failure) }}})
		if !errors.Is(err, failure) {
			t.Fatal("shutdown hid a cleanup failure", err)
		}
	}
}

func TestRunningServiceStopsAndDeletesOnlyOwnedApiHeartbeat(t *testing.T) {
	configuration := runtimeConfiguration(t)
	configuration.SchedulerIntervalSeconds = math.MaxUint64
	prepared, err := Prepare(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	if err := backup.RecordHeartbeat(context.Background(), configuration.Storage.AuthDbPath, backup.Api, "other-instance", unixTime()); err != nil {
		t.Fatal(err)
	}
	address := prepared.Address().String()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- prepared.Run(ctx) }()
	t.Cleanup(func() { cancel(); prepared.Close() })
	client := &http.Client{Timeout: 5 * time.Second}
	deadline := time.Now().Add(5 * time.Second)
	for {
		response, err := client.Get("http://" + address + "/health/ready")
		if err != nil {
			t.Fatal(err)
		}
		io.Copy(io.Discard, response.Body)
		response.Body.Close()
		if response.StatusCode == http.StatusOK {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("service never became ready", response.StatusCode)
		}
		time.Sleep(10 * time.Millisecond)
	}
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("shutdown did not join all components")
	}
	listener, err := net.Listen("tcp", address)
	if err != nil {
		t.Fatal("shutdown retained listener", err)
	}
	listener.Close()
	repository, err := auth.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	err = repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		var count int
		var instance string
		if err := connection.QueryRowContext(context.Background(), "SELECT count(*),min(instance_id) FROM service_heartbeats WHERE service='api'").Scan(&count, &instance); err != nil {
			return err
		}
		if count != 1 || instance != "other-instance" {
			t.Errorf("heartbeat cleanup removed others or retained self: %d %s", count, instance)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
