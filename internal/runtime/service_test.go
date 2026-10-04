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
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/backup"
	"github.com/QianFuv/LitRadar/internal/storage/maintenance"
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
	ctx, cancel := context.WithCancel(context.Background())
	started, release := make(chan struct{}), make(chan struct{})
	finished := make(chan error, 1)
	caller := make(chan error, 1)
	go func() {
		_, err := executor.Run(ctx, prepared.services.StoragePool, func() (struct{}, error) {
			close(started)
			<-release
			err := prepared.services.Auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
				var value int
				return connection.QueryRowContext(context.Background(), "SELECT 1").Scan(&value)
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
