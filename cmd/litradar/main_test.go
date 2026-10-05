package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"

	"runtime"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/testkit/fullstack"
)

func TestExecutableOwnsPublicCommandsAndLogging(t *testing.T) {
	library, err := sqlite.SimpleLibrary()
	if err != nil {
		t.Fatal(err)
	}
	native, err := os.ReadFile(library)
	if err != nil {
		t.Fatal(err)
	}
	nativeHash := fmt.Sprintf("%x", sha256.Sum256(native))
	identity, _ := json.Marshal(map[string]string{"path": library, "sha256": nativeHash})
	t.Logf("NATIVE_SIMPLE %s", identity)
	defer func() {
		after, err := os.ReadFile(library)
		if err != nil || fmt.Sprintf("%x", sha256.Sum256(after)) != nativeHash {
			t.Error("loaded native library changed during process proof", err)
		}
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	binary := filepath.Join(t.TempDir(), "litradar")
	if runtime.GOOS == "windows" {
		binary += ".exe"
	}
	compiler := filepath.Join(runtime.GOROOT(), "bin", "go")
	if runtime.GOOS == "windows" {
		compiler += ".exe"
	}
	build := exec.CommandContext(ctx, compiler, "build", "-mod=readonly", "-tags", "sqlite_fts5,sqlite_dbstat", "-o", binary, ".")
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatal(err, string(output))
	}
	working := t.TempDir()
	invoke := func(t *testing.T, input string, code int, args ...string) (string, string) {
		t.Helper()
		command := exec.CommandContext(ctx, binary, args...)
		command.Dir = working
		command.Stdin = strings.NewReader(input)
		var stdout, stderr bytes.Buffer
		command.Stdout = &stdout
		command.Stderr = &stderr
		err := command.Run()
		actual := 0
		if err != nil {
			if failure, ok := err.(*exec.ExitError); ok {
				actual = failure.ExitCode()
			} else {
				t.Fatal(err)
			}
		}
		if actual != code {
			t.Fatalf("%v: exit %d, want %d\n%s\n%s", args, actual, code, stdout.String(), stderr.String())
		}
		if strings.Contains(stderr.String(), "SyntheticProcessPassword!2026") {
			t.Fatal("password leaked")
		}
		return stdout.String(), stderr.String()
	}
	for _, args := range [][]string{{"--help"}, {"admin", "--help"}, {"index", "--help"}, {"notify", "--help"}} {
		stdout, stderr := invoke(t, "", 0, args...)
		if !strings.Contains(strings.ToLower(stdout), "usage") || !strings.Contains(stderr, `"event":"process.started"`) {
			t.Fatal("missing help or startup event", args, stdout, stderr)
		}
	}
	stdout, _ := invoke(t, "", 0, "openapi")
	var document map[string]any
	if err := json.Unmarshal([]byte(stdout), &document); err != nil || document["paths"] == nil {
		t.Fatal("missing API paths", err)
	}

	if _, err := os.Stat(filepath.Join(working, "data")); !os.IsNotExist(err) {
		t.Fatal("read-only commands created deployment data", err)
	}
	root := t.TempDir()
	stdout, _ = invoke(t, "SyntheticProcessPassword!2026\n", 0, "admin", "bootstrap", "--username", "process_admin", "--password-stdin", "--project-root", root)
	if !strings.Contains(stdout, `"status":"created"`) {
		t.Fatal(stdout)
	}
	key := filepath.Join(root, "secret.key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	for _, operation := range []string{"migrate", "verify"} {
		invoke(t, "", 0, "admin", "secrets", operation, "--secret-key-file", key, "--project-root", root)
	}
	backup := filepath.Join(root, "snapshot")
	invoke(t, "", 0, "admin", "backup", "create", "--output", backup, "--project-root", root)
	invoke(t, "", 0, "admin", "backup", "verify", "--backup", backup, "--project-root", root)
	invoke(t, "", 0, "admin", "backup", "restore", "--backup", backup, "--confirm-restore", "--project-root", root)
	invoke(t, "", 0, "scheduler", "validate", "--secret-key-file", key, "--project-root", root)
	fixture := t.TempDir()
	if err := os.WriteFile(filepath.Join(fixture, ".litradar-e2e-root"), []byte("litradar-full-stack-e2e-v1\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := fullstack.Run(ctx, []string{"--project-root", fixture}, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	for _, operation := range []string{"notify", "push"} {
		stdout, _ = invoke(t, "", 0, operation, "--dry-run", "--db", "full-stack.sqlite", "--secret-key-file", key, "--project-root", fixture)
		if !json.Valid([]byte(stdout)) {
			t.Fatal("delivery stdout is not JSON", stdout)
		}
	}
	if runtime.GOOS != "windows" {
		t.Run("real_sigterm", func(t *testing.T) { verifyServiceSignal(t, ctx, binary, working, fixture, key) })
		t.Run("blocked_stdin_signal", func(t *testing.T) { verifyBlockedInputSignal(t, ctx, binary, working, fixture) })
	}
	t.Run("same_binary_index_notify", func(t *testing.T) {
		indexed := prepareRecoveredIndex(t, ctx)
		stdout, stderr := invoke(t, "", 0, "index", "--project-root", indexed, "--secret-key-file", key, "--file", "fixture.csv", "--update", "--notify", "--notify-dry-run", "--workers", "1", "--processes", "2")
		verifyRecoveredIndex(t, indexed, stdout, stderr)
	})
	filename := filepath.Join(root, "data", "auth.sqlite")
	database, err := sqlite.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("INSERT INTO runtime_settings(key,value,updated_at) VALUES('log_format','compact',1),('log_filter','info',1)"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	_, stderr := invoke(t, "", 0, "admin", "--help", "--project-root", root)
	if !strings.Contains(stderr, `process:cli.command: litradar_cli:`) || strings.Contains(stderr, `process{`) {
		t.Fatal("persisted compact logging ignored", stderr)
	}
	database, err = sqlite.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("UPDATE runtime_settings SET value='synthetic-invalid-log-format' WHERE key='log_format'"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	stdout, stderr = invoke(t, "", 1, "--help", "--project-root", root)
	if stdout != "" || stderr != "invalid LitRadar log format\n" {
		t.Fatal("invalid logging did not fail closed with a fixed diagnostic", stdout, stderr)
	}
}

func verifyBlockedInputSignal(t *testing.T, ctx context.Context, binary, working, root string) {
	t.Helper()
	for _, termination := range []os.Signal{os.Interrupt, syscall.SIGTERM} {
		logfile, err := os.CreateTemp(t.TempDir(), "bootstrap-*.log")
		if err != nil {
			t.Fatal(err)
		}
		command := exec.CommandContext(ctx, binary, "admin", "bootstrap", "--username", "signal_admin", "--password-stdin", "--project-root", root)
		command.Dir, command.Stderr = working, logfile
		input, err := command.StdinPipe()
		if err != nil {
			t.Fatal(err)
		}
		if err := command.Start(); err != nil {
			t.Fatal(err)
		}
		finished := make(chan error, 1)
		go func() { finished <- command.Wait() }()
		deadline := time.Now().Add(5 * time.Second)
		isStarted := false
		for time.Now().Before(deadline) {
			data, _ := os.ReadFile(logfile.Name())
			if bytes.Contains(data, []byte(`"event":"cli.command.started"`)) {
				isStarted = true
				break
			}
			time.Sleep(20 * time.Millisecond)
		}
		time.Sleep(100 * time.Millisecond)
		if !isStarted {
			command.Process.Kill()
			<-finished
			input.Close()
			logfile.Close()
			t.Fatal("bootstrap did not start")
		}
		if err := command.Process.Signal(termination); err != nil {
			command.Process.Kill()
			<-finished
			input.Close()
			logfile.Close()
			t.Fatal(err)
		}
		select {
		case err := <-finished:
			failure, ok := err.(*exec.ExitError)
			if !ok || failure.ExitCode() != -1 {
				t.Errorf("bootstrap did not retain default %v termination: %v", termination, err)
			}
		case <-time.After(2 * time.Second):
			command.Process.Kill()
			<-finished
			t.Errorf("bootstrap swallowed %v while blocked on stdin", termination)
		}
		input.Close()
		logfile.Close()
	}
}

func verifyServiceSignal(t *testing.T, ctx context.Context, binary, working, root, key string) {
	t.Helper()
	logfile, err := os.CreateTemp(t.TempDir(), "service-*.log")
	if err != nil {
		t.Fatal(err)
	}
	defer logfile.Close()
	command := exec.CommandContext(ctx, binary, "serve", "--development", "--host", "127.0.0.1", "--port", "0", "--secret-key-file", key, "--project-root", root)
	command.Dir = working
	command.Stderr = logfile
	command.Stdout = logfile
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	isWaited := false
	defer func() {
		if !isWaited {
			_ = command.Process.Kill()
			_ = command.Wait()
		}
	}()
	deadline := time.Now().Add(15 * time.Second)
	port := 0
	for time.Now().Before(deadline) && port == 0 {
		data, _ := os.ReadFile(logfile.Name())
		for _, line := range bytes.Split(data, []byte{'\n'}) {
			var event struct {
				Event string
				Port  int
			}
			if json.Unmarshal(line, &event) == nil && event.Event == "service.listener.ready" {
				port = event.Port
			}
		}
		if port == 0 {
			time.Sleep(20 * time.Millisecond)
		}
	}
	if port == 0 {
		data, _ := os.ReadFile(logfile.Name())
		t.Fatal("listener never ready", string(data))
	}
	client := &http.Client{Timeout: 3 * time.Second}
	for {
		response, err := client.Get(fmt.Sprintf("http://127.0.0.1:%d/health/ready", port))
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, response.Body)
		response.Body.Close()
		if response.StatusCode == http.StatusOK {
			break
		}
		if time.Now().After(deadline) {
			data, _ := os.ReadFile(logfile.Name())
			t.Fatal("service never became ready", response.StatusCode, string(data))
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err := command.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	err = command.Wait()
	isWaited = true
	if err != nil {
		t.Fatal("signal shutdown failed", err)
	}
	data, _ := os.ReadFile(logfile.Name())
	for _, event := range []string{"service.signal.received", "service.shutdown.requested", "service.shutdown.completed", "service.stopped", "process.completed"} {
		if !bytes.Contains(data, []byte(`"event":"`+event+`"`)) {
			t.Fatal("missing shutdown event", event, string(data))
		}
	}
	database, err := sqlite.Open(filepath.Join(root, "data", "auth.sqlite"), true, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	var count int
	if err := database.QueryRow("SELECT count(*) FROM service_heartbeats WHERE service='api'").Scan(&count); err != nil || count != 0 {
		t.Fatal("signal left an API heartbeat", count, err)
	}
}
