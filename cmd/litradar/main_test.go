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
	"regexp"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/testkit/fullstack"
)

// TestEmbeddedWebProduction proves the tagged executable serves its real export from an empty project root.
func TestEmbeddedWebProduction(t *testing.T) {
	if os.Getenv("LITRADAR_TEST_WEB_ROOT") == "" {
		t.Skip("requires the independently built production export")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	binary := filepath.Join(t.TempDir(), "litradar")
	compiler := filepath.Join(runtime.GOROOT(), "bin", "go")
	if runtime.GOOS == "windows" {
		binary += ".exe"
		compiler += ".exe"
	}
	build := exec.CommandContext(ctx, compiler, "build", "-mod=readonly", "-trimpath", "-tags", "sqlite_fts5,sqlite_dbstat,litradar_web", "-o", binary, ".")
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatal(err, string(output))
	}
	root := t.TempDir()
	key := filepath.Join(root, "secret.key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	logfile, err := os.CreateTemp(t.TempDir(), "embedded-service-*.log")
	if err != nil {
		t.Fatal(err)
	}
	defer logfile.Close()
	command := exec.CommandContext(ctx, binary, "serve", "--host", "127.0.0.1", "--port", "0", "--secret-key-file", key, "--project-root", root)
	command.Dir, command.Stdout, command.Stderr = root, logfile, logfile
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { command.Process.Kill(); command.Wait() }()
	deadline := time.Now().Add(60 * time.Second)
	port := readServiceListenerPort(logfile, deadline)
	if port == 0 {
		data, _ := os.ReadFile(logfile.Name())
		t.Fatal("embedded production listener unavailable", string(data))
	}
	awaitServiceReadiness(t, logfile, port, deadline)
	base := fmt.Sprintf("http://127.0.0.1:%d", port)
	client := &http.Client{Timeout: 5 * time.Second}
	response, err := client.Get(base + "/")
	if err != nil {
		t.Fatal(err)
	}
	html, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != 200 || !strings.Contains(response.Header.Get("Content-Security-Policy"), "sha256-") || response.Header.Get("Last-Modified") != "" || response.Header.Get("ETag") == "" || response.Header.Get("Cache-Control") != "no-cache" {
		t.Fatal("embedded HTML/security validators unavailable", response.StatusCode, response.Header, err)
	}
	asset := regexp.MustCompile(`src="(/_next/static/[^" ]+\.js)"`).FindSubmatch(html)
	stylesheet := regexp.MustCompile(`href="(/_next/static/[^" ]+\.css)"`).FindSubmatch(html)
	if len(asset) != 2 || len(stylesheet) != 2 {
		t.Fatal("real _next script or stylesheet missing")
	}
	for _, endpoint := range []string{string(asset[1]), string(stylesheet[1]), "/login", "/openapi.json"} {
		response, err := client.Get(base + endpoint)
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		if response.StatusCode != 200 {
			t.Fatal(endpoint, response.StatusCode)
		}
		if strings.HasPrefix(endpoint, "/_next/") && !strings.Contains(response.Header.Get("Cache-Control"), "immutable") {
			t.Fatal("hashed asset cache contract", endpoint, response.Header)
		}
	}
	if _, err := os.Stat(filepath.Join(root, "web")); !os.IsNotExist(err) {
		t.Fatal("frontend assets were extracted", err)
	}
	missing, err := client.Get(base + "/missing-embedded-page")
	if err != nil {
		t.Fatal(err)
	}
	missing.Body.Close()
	if missing.StatusCode != 404 {
		t.Fatal("missing page", missing.StatusCode)
	}
	if err := os.Mkdir(filepath.Join(root, "web"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "web", "index.html"), []byte("external override"), 0600); err != nil {
		t.Fatal(err)
	}
	override, err := client.Get(base + "/")
	if err != nil {
		t.Fatal(err)
	}
	actual, err := io.ReadAll(override.Body)
	override.Body.Close()
	if err != nil || !bytes.Equal(actual, html) {
		t.Fatal("external web tree changed embedded response", err)
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

// buildProcessExecutable preserves the corresponding executable proof phase.
func buildProcessExecutable(t *testing.T, ctx context.Context) string {
	t.Helper()
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
