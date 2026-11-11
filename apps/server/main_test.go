package main

import (
	"bytes"
	"context"
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

	"github.com/QianFuv/LitRadar"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/fullstack"
)

// TestServerProductionWithoutUI proves production starts from an empty project root.
func TestServerProductionWithoutUI(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	binary := buildProcessExecutable(t, ctx)
	root := t.TempDir()
	key := filepath.Join(root, "secret.key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	bootstrap := exec.CommandContext(ctx, binary, "admin", "bootstrap", "--username", "production_admin", "--password-stdin", "--project-root", root)
	bootstrap.Dir = t.TempDir()
	bootstrap.Stdin = strings.NewReader("SyntheticProduction!2026\n")
	if output, err := bootstrap.CombinedOutput(); err != nil {
		t.Fatal(err, string(output))
	}
	logfile, err := os.CreateTemp(t.TempDir(), "server-service-*.log")
	if err != nil {
		t.Fatal(err)
	}
	defer logfile.Close()
	working := t.TempDir()
	command := exec.CommandContext(ctx, binary, "serve", "--host", "127.0.0.1", "--port", "0", "--secret-key-file", key, "--project-root", root)
	command.Dir, command.Stdout, command.Stderr = working, logfile, logfile
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { command.Process.Kill(); command.Wait() }()
	deadline := time.Now().Add(60 * time.Second)
	port := readServiceListenerPort(logfile, deadline)
	if port == 0 {
		data, _ := os.ReadFile(logfile.Name())
		t.Fatal("production listener unavailable", string(data))
	}
	awaitServiceReadiness(t, logfile, port, deadline)
	base := fmt.Sprintf("http://127.0.0.1:%d", port)
	client := &http.Client{Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	for _, scenario := range []struct {
		path   string
		status int
	}{
		{"/", 404}, {"/login", 404}, {"/missing-page", 404}, {"/page/", 404}, {"/_next/static/asset.js", 404},
		{"/health/ready", 200}, {"/docs", 200}, {"/docs/", 303}, {"/openapi.json", 200}, {"/api/auth/me", 401},
	} {
		response, err := client.Get(base + scenario.path)
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(response.Body)
		response.Body.Close()
		if err != nil || response.StatusCode != scenario.status || response.Header.Get("Content-Security-Policy") == "" {
			t.Fatal(scenario.path, response.StatusCode, response.Header, err)
		}
		if scenario.status == 404 && bytes.Contains(body, []byte("<html")) {
			t.Fatal("unexpected UI fallback", scenario.path)
		}
	}
	login, err := client.Post(base+"/api/auth/login", "application/json", strings.NewReader(`{"username":"production_admin","password":"SyntheticProduction!2026"}`))
	if err != nil {
		t.Fatal(err)
	}
	login.Body.Close()
	if login.StatusCode != 200 || len(login.Cookies()) != 1 {
		t.Fatal("production cookie login failed", login.StatusCode)
	}
	for _, isAuthenticated := range []bool{false, true} {
		request, err := http.NewRequest("POST", base+"/mcp", strings.NewReader(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"production-smoke","version":"1"}}}`))
		if err != nil {
			t.Fatal(err)
		}
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "application/json, text/event-stream")
		if isAuthenticated {
			request.AddCookie(login.Cookies()[0])
		}
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		expected := 401
		if isAuthenticated {
			expected = 200
		}
		if response.StatusCode != expected {
			t.Fatal("production MCP authentication", isAuthenticated, response.StatusCode)
		}
	}
	if _, err := os.Stat(filepath.Join(working, "data")); !os.IsNotExist(err) {
		t.Fatal("project root leaked into working directory", err)
	}
}

// TestExecutableOwnsPublicCommandsAndLogging proves public command behavior in the built executable.
func TestExecutableOwnsPublicCommandsAndLogging(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	binary := buildProcessExecutable(t, ctx)
	working := t.TempDir()
	assertProcessReleaseIdentity(t, ctx, binary, working)
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
	assertReadOnlyProcessCommands(t, working, invoke)
	root, key := prepareProcessAdministrator(t, invoke)
	fixture := prepareProcessFullStack(t, ctx, key, invoke)
	if runtime.GOOS != "windows" {
		t.Run("real_sigterm", func(t *testing.T) { verifyServiceSignal(t, ctx, binary, working, fixture, key) })
		t.Run("blocked_stdin_signal", func(t *testing.T) { verifyBlockedInputSignal(t, ctx, binary, working, fixture) })
	}
	t.Run("same_binary_index_notify", func(t *testing.T) {
		indexed := prepareRecoveredIndex(t, ctx)
		stdout, stderr := invoke(t, "", 0, "index", "--project-root", indexed, "--secret-key-file", key, "--file", "fixture.csv", "--update", "--notify", "--notify-dry-run", "--workers", "1", "--processes", "2")
		verifyRecoveredIndex(t, indexed, stdout, stderr)
	})
	assertPersistedProcessLogging(t, root, invoke)

}

// verifyBlockedInputSignal proves both default termination signals during blocked input.
func verifyBlockedInputSignal(t *testing.T, ctx context.Context, binary, working, root string) {
	t.Helper()
	for _, termination := range []os.Signal{os.Interrupt, syscall.SIGTERM} {
		verifyBlockedSignalTrial(t, ctx, binary, working, root, termination)
	}
}

// verifyServiceSignal retains ownership until graceful service shutdown is joined.
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
	port := readServiceListenerPort(logfile, deadline)
	if port == 0 {
		data, _ := os.ReadFile(logfile.Name())
		t.Fatal("listener never ready", string(data))
	}
	awaitServiceReadiness(t, logfile, port, deadline)
	if err := command.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	err = command.Wait()
	isWaited = true
	if err != nil {
		t.Fatal("signal shutdown failed", err)
	}
	assertJoinedServiceShutdown(t, logfile, root)
}

// processInvocation executes a public command with its expected exit status.
type processInvocation func(*testing.T, string, int, ...string) (string, string)

// buildProcessExecutable builds the public executable without SQLite build tags.
func buildProcessExecutable(t *testing.T, ctx context.Context) string {
	t.Helper()
	binary := filepath.Join(t.TempDir(), "litradar")
	if runtime.GOOS == "windows" {
		binary += ".exe"
	}
	compiler, err := exec.LookPath("go")
	if err != nil {
		t.Fatal(err)
	}
	build := exec.CommandContext(ctx, compiler, "build", "-mod=readonly", "-o", binary, ".")
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatal(err, string(output))
	}
	return binary
}

// assertProcessReleaseIdentity preserves the corresponding executable proof phase.
func assertProcessReleaseIdentity(t *testing.T, ctx context.Context, binary, working string) {
	t.Helper()
	version := exec.CommandContext(ctx, binary, "--version")
	version.Dir = working
	if output, err := version.CombinedOutput(); err != nil || string(output) != "litradar "+litradar.Version()+"\n" {
		t.Fatalf("release identity: %q, %v", output, err)
	}
	if entries, err := os.ReadDir(working); err != nil || len(entries) != 0 {
		t.Fatalf("version command created runtime state: %v, %v", entries, err)
	}
}

// assertReadOnlyProcessCommands preserves the corresponding executable proof phase.
func assertReadOnlyProcessCommands(t *testing.T, working string, invoke processInvocation) {
	t.Helper()
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
}

// prepareProcessAdministrator preserves the corresponding executable proof phase.
func prepareProcessAdministrator(t *testing.T, invoke processInvocation) (string, string) {
	t.Helper()
	root := t.TempDir()
	stdout, _ := invoke(t, "SyntheticProcessPassword!2026\n", 0, "admin", "bootstrap", "--username", "process_admin", "--password-stdin", "--project-root", root)
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
	return root, key
}

// prepareProcessFullStack preserves the corresponding executable proof phase.
func prepareProcessFullStack(t *testing.T, ctx context.Context, key string, invoke processInvocation) string {
	t.Helper()
	fixture := t.TempDir()
	if err := os.WriteFile(filepath.Join(fixture, ".litradar-e2e-root"), []byte("litradar-full-stack-e2e-v1\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := fullstack.Run(ctx, []string{"--project-root", fixture}, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	for _, operation := range []string{"notify", "push"} {
		stdout, _ := invoke(t, "", 0, operation, "--dry-run", "--db", "full-stack.sqlite", "--secret-key-file", key, "--project-root", fixture)
		if !json.Valid([]byte(stdout)) {
			t.Fatal("delivery stdout is not JSON", stdout)
		}
	}
	return fixture
}

// assertPersistedProcessLogging preserves the corresponding executable proof phase.
func assertPersistedProcessLogging(t *testing.T, root string, invoke processInvocation) {
	t.Helper()
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
	stdout, stderr := invoke(t, "", 1, "--help", "--project-root", root)
	if stdout != "" || stderr != "invalid LitRadar log format\n" {
		t.Fatal("invalid logging did not fail closed with a fixed diagnostic", stdout, stderr)
	}
}

// verifyBlockedSignalTrial preserves default termination while bootstrap reads stdin.
func verifyBlockedSignalTrial(t *testing.T, ctx context.Context, binary, working, root string, termination os.Signal) {
	t.Helper()
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
	isStarted := awaitBlockedBootstrapStart(logfile.Name())
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

// awaitBlockedBootstrapStart waits for the command admission event.
func awaitBlockedBootstrapStart(filename string) bool {
	deadline := time.Now().Add(5 * time.Second)
	isStarted := false
	for time.Now().Before(deadline) {
		data, _ := os.ReadFile(filename)
		if bytes.Contains(data, []byte(`"event":"cli.command.started"`)) {
			isStarted = true
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	return isStarted
}

// readServiceListenerPort waits for the listener announcement within the shared deadline.
func readServiceListenerPort(logfile *os.File, deadline time.Time) int {
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
	return port
}

// awaitServiceReadiness preserves the corresponding executable proof phase.
func awaitServiceReadiness(t *testing.T, logfile *os.File, port int, deadline time.Time) {
	t.Helper()
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
}

// assertJoinedServiceShutdown checks joined shutdown events and heartbeat removal.
func assertJoinedServiceShutdown(t *testing.T, logfile *os.File, root string) {
	t.Helper()
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
