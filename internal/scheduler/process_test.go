package scheduler

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
)

func TestMain(tests *testing.M) {
	role := os.Getenv("LITRADAR_SCHEDULER_TEST_ROLE")
	if role == "" {
		os.Exit(tests.Run())
	}
	directory := os.Getenv("LITRADAR_SCHEDULER_TEST_DIRECTORY")
	switch role {
	case "claim-crash":
		runClaimCrashFixture(directory)
	case "exit":
		code, _ := strconv.Atoi(os.Getenv("LITRADAR_SCHEDULER_TEST_EXIT"))
		os.Exit(code)
	case "output":
		os.Stdout.Write(bytes.Repeat([]byte("x"), 2047))
		os.Stdout.Write([]byte{0xe4, 0xb8, 0xad})
		os.Stdout.Write(bytes.Repeat([]byte("y"), 100000))
		fmt.Fprint(os.Stderr, "scheduler-private-diagnostic")
		os.Exit(0)
	case "sequence":
		runProcessSequenceFixture(directory)
	case "tree", "leader-exit":
		runProcessTreeFixture(directory, role)
	case "descendant":
		runProcessDescendantFixture(directory)
	default:
		os.Exit(35)
	}
}

func processFixture(t *testing.T, role string) (scheduledProcess, string) {
	t.Helper()
	t.Setenv("GORACE", os.Getenv("GORACE")+" atexit_sleep_ms=0")
	directory := t.TempDir()
	t.Setenv("LITRADAR_SCHEDULER_TEST_ROLE", role)
	t.Setenv("LITRADAR_SCHEDULER_TEST_DIRECTORY", directory)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return scheduledProcess{command: "index", path: executable}, directory
}
func testClaim() store.Claim {
	return store.Claim{RunId: 42, WorkerId: "fixture", Task: domain.Task{Id: 1, Name: "fixture"}}
}

func TestSchedulerWindowsSignedExitCode(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Windows u32 process status")
	}
	command, _ := processFixture(t, "exit")
	for _, code := range []int{-1, -1073741819} {
		t.Setenv("LITRADAR_SCHEDULER_TEST_EXIT", strconv.Itoa(code))
		result := executeProcess(context.Background(), command, testClaim(), 1, time.Now().Add(5*time.Second), time.Second, func() bool { return true })
		if result.status != domain.Failed || result.summary != fmt.Sprintf("index: exit code %d", code) {
			t.Fatalf("%d: %#v", code, result)
		}
	}
}

func TestSchedulerOutputBoundedAndRedacted(t *testing.T) {
	command, _ := processFixture(t, "output")
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	result := executeProcess(context.Background(), command, testClaim(), 1, time.Now().Add(5*time.Second), time.Second, func() bool { return true })
	expected := "stdout: " + strings.Repeat("x", 2047) + "�"
	if result.status != domain.Success || result.summary != expected {
		t.Fatalf("unexpected captured output: %s length=%d", result.status, len(result.summary))
	}
	for _, secret := range []string{command.path, "scheduler-private-diagnostic", strings.Repeat("x", 20)} {
		if strings.Contains(logs.String(), secret) {
			t.Fatalf("private output leaked to event log")
		}
	}
	if strings.Count(logs.String(), `"event":"scheduler.child.completed"`) != 1 {
		t.Fatal(logs.String())
	}
}

func TestSchedulerSpawnFailureIsSingleAndRedacted(t *testing.T) {
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	command := scheduledProcess{command: "index", path: filepath.Join(t.TempDir(), "private-missing-executable")}
	result := executeProcess(context.Background(), command, testClaim(), 1, time.Now().Add(time.Second), time.Second, func() bool { t.Fatal("spawn failure invoked heartbeat"); return false })
	if result.status != domain.Error || result.summary != "index: process supervision failed (spawn_or_assign_failed)" {
		t.Fatalf("%#v", result)
	}
	if strings.Contains(logs.String(), command.path) || strings.Count(logs.String(), `"event":"scheduler.child.failed"`) != 1 {
		t.Fatal("invalid or unsafe terminal event")
	}
	for _, field := range []string{`"worker_id":"fixture"`, `"task_id":1`, `"run_id":"42"`, `"job_id":"scheduled-task-1"`, `"error_kind":"spawn_or_assign_failed"`} {
		if !strings.Contains(logs.String(), field) {
			t.Fatalf("missing log context %s", field)
		}
	}
}

func waitDescendant(t *testing.T, directory string) string {
	t.Helper()
	for deadline := time.Now().Add(8 * time.Second); time.Now().Before(deadline); {
		if data, err := os.ReadFile(filepath.Join(directory, "ready")); err == nil {
			return string(data)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("descendant did not bind")
	return ""
}
func assertDescendantStopped(t *testing.T, address string) {
	t.Helper()
	connection, err := net.DialTimeout("tcp", address, 200*time.Millisecond)
	if err == nil {
		connection.Close()
		t.Fatal("descendant survived tree cleanup")
	}
}

func TestSchedulerTreeTermination(t *testing.T) {
	for _, reason := range []string{"cancel", "timeout", "heartbeat", "leader-exit"} {
		t.Run(reason, func(t *testing.T) {
			role := "tree"
			if reason == "leader-exit" {
				role = reason
			}
			command, directory := processFixture(t, role)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			deadline := time.Now().Add(5 * time.Second)
			if reason == "timeout" {
				deadline = time.Now().Add(1200 * time.Millisecond)
			}
			resultChannel := make(chan processResult, 1)
			go func() {
				resultChannel <- executeProcess(ctx, command, testClaim(), 1, deadline, 1200*time.Millisecond, func() bool { return reason != "heartbeat" })
			}()
			address := waitDescendant(t, directory)
			if reason == "cancel" {
				cancel()
			}
			select {
			case result := <-resultChannel:
				expected := map[string]domain.State{"cancel": domain.Cancelled, "timeout": domain.TimedOut, "heartbeat": domain.Unknown, "leader-exit": domain.Success}[reason]
				if result.status != expected {
					t.Fatalf("%s: %#v", reason, result)
				}
			case <-time.After(12 * time.Second):
				t.Fatal("supervisor did not finish")
			}
			assertDescendantStopped(t, address)
		})
	}
}

func TestSchedulerCommandsSequenceAndSharedDeadline(t *testing.T) {
	for _, scenario := range []string{"success", "first-failed", "deadline"} {
		t.Run(scenario, func(t *testing.T) {
			command, directory := processFixture(t, "sequence")
			if scenario == "first-failed" {
				t.Setenv("LITRADAR_SCHEDULER_TEST_FAIL", "index")
			}
			if scenario == "deadline" {
				t.Setenv("LITRADAR_SCHEDULER_TEST_DELAY", "yes")
			}
			task := domain.Task{Id: 1, Job: &domain.Job{Kind: "index", Notify: true, Push: true}, TimeoutSeconds: 5}
			if scenario == "deadline" {
				task.TimeoutSeconds = 15
			}
			config := ProcessConfig{ProjectRoot: "explicit root with spaces", AuthDatabase: "auth path", Executable: command.path, SecretKeyFile: "key path"}
			started := time.Now()
			result := config.runProcesses(context.Background(), task, testClaim(), func() bool { return true })
			if scenario == "deadline" && time.Since(started) > 18500*time.Millisecond {
				t.Fatal("deadline was reset between commands")
			}
			expected := map[string]domain.State{"success": domain.Success, "first-failed": domain.Failed, "deadline": domain.TimedOut}[scenario]
			if result.status != expected {
				t.Fatalf("%#v", result)
			}
			data, err := os.ReadFile(filepath.Join(directory, "commands.jsonl"))
			if err != nil {
				t.Fatal(err)
			}
			lines := bytes.Split(bytes.TrimSpace(data), []byte("\n"))
			count := map[string]int{"success": 3, "first-failed": 1, "deadline": 2}[scenario]
			if len(lines) != count {
				t.Fatalf("ran %d commands, want %d", len(lines), count)
			}
			cwd, _ := os.Getwd()
			for index, line := range lines {
				var record struct {
					Args []string
					Cwd  string
				}
				json.Unmarshal(line, &record)
				if record.Cwd != cwd || record.Args[0] != []string{"index", "notify", "push"}[index] || strings.Join(record.Args[len(record.Args)-2:], " ") != ParentRunIdArgument+" 42" {
					t.Fatalf("wrong command %#v", record)
				}
			}
		})
	}
}

// runClaimCrashFixture preserves durable crash boundaries before publishing the barrier and hanging.
func runClaimCrashFixture(directory string) {
	boundary := os.Getenv("LITRADAR_SCHEDULER_TEST_BOUNDARY")
	repository, err := store.Open(filepath.Join(directory, "auth.sqlite"))
	if err != nil {
		os.Exit(40)
	}
	claims, err := repository.ClaimReady(context.Background(), "crashed", 100, 90, 1)
	if err != nil || len(claims) != 1 {
		os.Exit(41)
	}
	if boundary != "claimed" {
		writeStartedCrashEffect(repository, claims[0], directory)
	}
	if boundary == "finished" {
		if finished, err := repository.FinishRun(context.Background(), claims[0], domain.Success, "complete", 101); err != nil || !finished {
			os.Exit(44)
		}
	}
	repository.Close()
	os.WriteFile(filepath.Join(directory, "barrier"), []byte(boundary), 0600)
	select {}
}

// writeStartedCrashEffect starts the owned claim before writing, syncing and closing its external effect.
func writeStartedCrashEffect(repository *store.Repository, claim store.Claim, directory string) {
	if started, err := repository.StartRun(context.Background(), claim.RunId, "crashed", 100, 90); err != nil || !started {
		os.Exit(42)
	}
	file, err := os.OpenFile(filepath.Join(directory, "effect"), os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		os.Exit(43)
	}
	file.WriteString("executed")
	file.Sync()
	file.Close()
}

// runProcessSequenceFixture records exact argv and working directory before delay or selected exit.
func runProcessSequenceFixture(directory string) {
	file, err := os.OpenFile(filepath.Join(directory, "commands.jsonl"), os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		os.Exit(30)
	}
	cwd, _ := os.Getwd()
	body, _ := json.Marshal(map[string]any{"args": os.Args[1:], "cwd": cwd})
	file.Write(append(body, '\n'))
	file.Sync()
	file.Close()
	if os.Getenv("LITRADAR_SCHEDULER_TEST_DELAY") == "yes" {
		time.Sleep(8 * time.Second)
	}
	if os.Getenv("LITRADAR_SCHEDULER_TEST_FAIL") == os.Args[1] {
		os.Exit(7)
	}
	os.Exit(0)
}

// runProcessTreeFixture preserves inherited output and readiness before leader exit or tree lifetime.
func runProcessTreeFixture(directory, role string) {
	command := exec.Command(os.Args[0])
	command.Env = []string{}
	for _, value := range os.Environ() {
		if !strings.HasPrefix(value, "LITRADAR_SCHEDULER_TEST_ROLE=") {
			command.Env = append(command.Env, value)
		}
	}
	command.Env = append(command.Env, "LITRADAR_SCHEDULER_TEST_ROLE=descendant")
	command.Stdout = os.Stdout
	command.Stderr = os.Stderr
	if command.Start() != nil {
		os.Exit(31)
	}
	for deadline := time.Now().Add(8 * time.Second); time.Now().Before(deadline); {
		if _, err := os.Stat(filepath.Join(directory, "ready")); err == nil {
			if role == "leader-exit" {
				os.Exit(0)
			}
			select {}
		}
		time.Sleep(10 * time.Millisecond)
	}
	os.Exit(32)
}

// runProcessDescendantFixture publishes the bound socket and serves until forced process cleanup.
func runProcessDescendantFixture(directory string) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(33)
	}
	os.WriteFile(filepath.Join(directory, "ready"), []byte(listener.Addr().String()), 0600)
	for {
		connection, err := listener.Accept()
		if err != nil {
			os.Exit(34)
		}
		connection.Close()
	}
}
