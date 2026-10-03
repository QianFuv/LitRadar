package process

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

type fixtureInfo struct {
	Pid     int
	Address string
}

func fixtureConfig(mode, directory string) Config {
	executable, err := os.Executable()
	if err != nil {
		panic(err)
	}
	environment := []string{}
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "LITRADAR_PROCESS_FIXTURE=") && !strings.HasPrefix(entry, "LITRADAR_PROCESS_DIRECTORY=") && !strings.HasPrefix(entry, ParentEnvironment+"=") {
			environment = append(environment, entry)
		}
	}
	environment = append(environment, "LITRADAR_PROCESS_FIXTURE="+mode, "LITRADAR_PROCESS_DIRECTORY="+directory)
	return Config{Path: executable, Args: []string{"-test.run=^TestProcessFixture$"}, Environment: environment, OutputLimit: 1024}
}

func fixtureCommand(mode, directory string) *exec.Cmd {
	config := fixtureConfig(mode, directory)
	command := exec.Command(config.Path, config.Args...)
	command.Env = config.Environment
	return command
}

func TestProcessFixture(t *testing.T) {
	mode := os.Getenv("LITRADAR_PROCESS_FIXTURE")
	if mode == "" {
		return
	}
	directory := os.Getenv("LITRADAR_PROCESS_DIRECTORY")
	if mode == "stderr-flood" {
		os.Stderr.Write(bytes.Repeat([]byte("e"), 8192))
		fmt.Fprint(os.Stdout, "result")
		os.Exit(0)
	}
	if mode == "protocol" {
		fmt.Fprintln(os.Stderr, "synthetic diagnostic")
		fmt.Fprintln(os.Stdout, `{"frame":"ready"}`)
		line, err := bufio.NewReader(os.Stdin).ReadString('\n')
		if err != nil || line != "ACK\n" {
			os.Exit(8)
		}
		fmt.Fprintln(os.Stdout, `{"frame":"acknowledged"}`)
		os.Exit(0)
	}
	if mode == "ignore-term" {
		signal.Ignore(syscall.SIGTERM)
	}
	if mode == "output" {
		for count := 0; count < 256; count++ {
			if _, err := os.Stdout.Write(bytes.Repeat([]byte("x"), 8192)); err != nil {
				os.Exit(3)
			}
		}
		os.Exit(0)
	}
	if mode == "owner" || mode == "owner-late-guard" {
		workerMode := "parent"
		if mode == "owner-late-guard" {
			workerMode = "late-guard"
		}
		config := fixtureConfig(workerMode, directory)
		config.Environment = append(config.Environment, ParentEnvironment+"="+strconv.Itoa(os.Getpid()))
		child, err := Start(context.Background(), config)
		if err != nil {
			panic(err)
		}
		defer child.Close()
		os.WriteFile(filepath.Join(directory, "owner.ready"), []byte(strconv.Itoa(os.Getpid())), 0600)
		select {}
	}
	if os.Getenv(ParentEnvironment) != "" && mode != "late-guard" {
		stop, err := StartParentGuard()
		if err != nil {
			os.Exit(4)
		}
		defer stop()
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		panic(err)
	}
	defer listener.Close()
	data, _ := json.Marshal(fixtureInfo{Pid: os.Getpid(), Address: listener.Addr().String()})
	if err := os.WriteFile(filepath.Join(directory, mode+".json"), data, 0600); err != nil {
		panic(err)
	}
	if mode == "parent" || mode == "leader-exit" || mode == "term-parent" || mode == "detached-pipes" || mode == "late-guard" {
		grandMode := "grandchild"
		if mode == "term-parent" {
			grandMode = "ignore-term"
		}
		grandchild := fixtureCommand(grandMode, directory)
		grandchild.Stdin, grandchild.Stdout, grandchild.Stderr = os.Stdin, os.Stdout, os.Stderr
		if mode == "detached-pipes" {
			grandchild.Stdin, grandchild.Stdout, grandchild.Stderr = nil, nil, nil
		}
		if err := grandchild.Start(); err != nil {
			panic(err)
		}
		if mode == "leader-exit" || mode == "detached-pipes" {
			os.Exit(0)
		}
	}
	if mode == "late-guard" {
		for {
			if _, err := os.Stat(filepath.Join(directory, "release-guard")); err == nil {
				break
			}
			time.Sleep(5 * time.Millisecond)
		}
		stop, err := StartParentGuard()
		if err != nil {
			os.Exit(4)
		}
		defer stop()
		os.WriteFile(filepath.Join(directory, "work-started"), []byte("unexpected"), 0600)
	}
	_, _ = io.Copy(io.Discard, os.Stdin)
	for {
		connection, err := listener.Accept()
		if err != nil {
			return
		}
		connection.Close()
	}
}

func TestCancellationAfterLeaderExitStillOwnsDescendants(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	directory := t.TempDir()
	child, err := Start(ctx, fixtureConfig("leader-exit", directory))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	grandchild := awaitFixture(t, directory, "grandchild")
	waitCtx, stop := context.WithTimeout(context.Background(), 3*time.Second)
	defer stop()
	if err := child.Wait(waitCtx); err != nil {
		t.Fatal(err)
	}
	requireReachable(t, grandchild)
	cancel()
	select {
	case <-child.closed:
	case <-waitCtx.Done():
		t.Fatal("cancel did not close tree ownership")
	}
	connection, err := net.DialTimeout("tcp", grandchild.Address, time.Second)
	if err == nil {
		connection.Close()
		t.Fatal("Close returned before descendant stopped")
	}
}

func TestProtocolStdoutIsSeparateAndBidirectional(t *testing.T) {
	config := fixtureConfig("protocol", t.TempDir())
	config.StreamStdout = true
	child, err := Start(context.Background(), config)
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	reader := bufio.NewReader(child.Stdout)
	ready, err := reader.ReadString('\n')
	if err != nil || ready != "{\"frame\":\"ready\"}\n" {
		t.Fatalf("protocol polluted: %q %v", ready, err)
	}
	if _, err := io.WriteString(child.Stdin, "ACK\n"); err != nil {
		t.Fatal(err)
	}
	ack, err := reader.ReadString('\n')
	if err != nil || !strings.Contains(ack, "acknowledged") {
		t.Fatalf("ACK protocol: %q %v", ack, err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := child.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	if err := child.Close(); err != nil {
		t.Fatal(err)
	}
	output, _ := child.Diagnostics()
	if !strings.Contains(string(output), "synthetic diagnostic") || strings.Contains(string(output), "frame") {
		t.Fatalf("diagnostic capture polluted: %q", output)
	}
}

func TestStderrCannotConsumeStdoutCapacity(t *testing.T) {
	child, err := Start(context.Background(), fixtureConfig("stderr-flood", t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := child.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	if err := child.Close(); err != nil {
		t.Fatal(err)
	}
	stdout, truncated := child.Output()
	stderr, diagnosticsTruncated := child.Diagnostics()
	if string(stdout) != "result" || truncated || len(stderr) != 1024 || !diagnosticsTruncated {
		t.Fatalf("stdout %q %v stderr %d %v", stdout, truncated, len(stderr), diagnosticsTruncated)
	}
}

func TestTreeWaitDoesNotDependOnOutputPipes(t *testing.T) {
	directory := t.TempDir()
	child, err := Start(context.Background(), fixtureConfig("detached-pipes", directory))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	grandchild := awaitFixture(t, directory, "grandchild")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := child.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-child.drained:
	case <-ctx.Done():
		t.Fatal("output still held by detached descendant")
	}
	requireReachable(t, grandchild)
	short, stop := context.WithTimeout(context.Background(), 30*time.Millisecond)
	err = child.tree.waitEmpty(short)
	stop()
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("live tree falsely empty: %v", err)
	}
	if err := child.Close(); err != nil {
		t.Fatal(err)
	}
	if connection, err := net.DialTimeout("tcp", grandchild.Address, time.Second); err == nil {
		connection.Close()
		t.Fatal("cleanup returned with live detached descendant")
	}
}

func TestPartialStartupFailureReapsOwnedTree(t *testing.T) {
	stages := []string{"after_resume"}
	if runtime.GOOS == "windows" {
		stages = append(stages, "before_assignment", "before_resume")
	}
	for _, stage := range stages {
		t.Run(stage, func(t *testing.T) {
			directory := t.TempDir()
			var grandchild fixtureInfo
			var pid int
			child, err := startWithHook(context.Background(), fixtureConfig("parent", directory), func(current string, command *exec.Cmd) error {
				if current != stage {
					return nil
				}
				pid = command.Process.Pid
				if stage == "after_resume" {
					grandchild = awaitFixture(t, directory, "grandchild")
					requireReachable(t, grandchild)
				}
				return errors.New("injected startup failure")
			})
			if child != nil || err == nil || err.Error() != "spawn_or_assign_failed" || pid == 0 {
				t.Fatalf("fault not exercised: %v %v %d", child, err, pid)
			}
			if stage != "after_resume" {
				if _, err := os.Stat(filepath.Join(directory, "parent.json")); !os.IsNotExist(err) {
					t.Fatal("unassigned or suspended child executed user code")
				}
			} else {
				requireStopped(t, grandchild)
			}
		})
	}
}

func TestGracefulAndForcedTreeTermination(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("SIGTERM grace is Unix-specific; Windows Job tests cover forced cleanup")
	}
	for _, scenario := range []struct {
		mode, descendant string
		want             Termination
	}{{"parent", "grandchild", Graceful}, {"term-parent", "ignore-term", Forced}} {
		t.Run(scenario.mode, func(t *testing.T) {
			directory := t.TempDir()
			child, err := Start(context.Background(), fixtureConfig(scenario.mode, directory))
			if err != nil {
				t.Fatal(err)
			}
			defer child.Close()
			grandchild := awaitFixture(t, directory, scenario.descendant)
			mode, err := child.Terminate(200 * time.Millisecond)
			if err != nil || mode != scenario.want {
				t.Fatalf("termination %s %v", mode, err)
			}
			connection, err := net.DialTimeout("tcp", grandchild.Address, time.Second)
			if err == nil {
				connection.Close()
				t.Fatal("termination returned before descendant stopped")
			}
		})
	}
}

func awaitFixture(t *testing.T, directory, mode string) fixtureInfo {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		data, err := os.ReadFile(filepath.Join(directory, mode+".json"))
		var info fixtureInfo
		if err == nil && json.Unmarshal(data, &info) == nil {
			return info
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("fixture %s not published", mode)
	return fixtureInfo{}
}

func requireReachable(t *testing.T, info fixtureInfo) {
	t.Helper()
	connection, err := net.DialTimeout("tcp", info.Address, time.Second)
	if err != nil {
		t.Fatalf("fixture not reachable: %v", err)
	}
	connection.Close()
}

func requireStopped(t *testing.T, info fixtureInfo) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		connection, err := net.DialTimeout("tcp", info.Address, 100*time.Millisecond)
		if err != nil {
			return
		}
		connection.Close()
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("descendant still listening: %+v", info)
}

func TestWholeTreeCleanupAfterLeaderExit(t *testing.T) {
	for _, mode := range []string{"parent", "leader-exit"} {
		t.Run(mode, func(t *testing.T) {
			directory := t.TempDir()
			child, err := Start(context.Background(), fixtureConfig(mode, directory))
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { child.Close() })
			parent := awaitFixture(t, directory, mode)
			grandchild := awaitFixture(t, directory, "grandchild")
			requireReachable(t, grandchild)
			if mode == "leader-exit" {
				ctx, cancel := context.WithTimeout(context.Background(), time.Second)
				defer cancel()
				if err := child.Wait(ctx); err != nil {
					t.Fatalf("leader wait blocked on descendant pipe: %v", err)
				}
			}
			var group sync.WaitGroup
			for range 4 {
				group.Go(func() {
					if err := child.Close(); err != nil {
						t.Error(err)
					}
				})
			}
			group.Wait()
			requireStopped(t, parent)
			requireStopped(t, grandchild)
		})
	}
}

func TestOutputLimitStillDrainsAndLeaderWaitPreservesPipe(t *testing.T) {
	child, err := Start(context.Background(), fixtureConfig("output", t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := child.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-child.drained:
	case <-ctx.Done():
		t.Fatal("output did not drain")
	}
	output, truncated := child.Output()
	if len(output) != 1024 || !truncated || !bytes.Equal(output, bytes.Repeat([]byte("x"), 1024)) {
		t.Fatalf("retained output %d truncated %v", len(output), truncated)
	}
}

func TestContextCancellationAndFailedStart(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	directory := t.TempDir()
	child, err := Start(ctx, fixtureConfig("parent", directory))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	grandchild := awaitFixture(t, directory, "grandchild")
	cancel()
	requireStopped(t, grandchild)
	if _, err := Start(ctx, fixtureConfig("output", directory)); err != context.Canceled {
		t.Fatalf("cancelled start: %v", err)
	}
	config := fixtureConfig("output", directory)
	config.Path = filepath.Join(directory, "missing-secret-executable")
	if _, err := Start(context.Background(), config); err == nil || err.Error() != "spawn_or_assign_failed" {
		t.Fatalf("failed start leaks path: %v", err)
	}
}

func TestOwnerDeathCleansBlockedTree(t *testing.T) {
	directory := t.TempDir()
	owner := fixtureCommand("owner", directory)
	owner.Stdout, owner.Stderr = io.Discard, io.Discard
	if err := owner.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { owner.Process.Kill(); owner.Wait() })
	parent := awaitFixture(t, directory, "parent")
	grandchild := awaitFixture(t, directory, "grandchild")
	requireReachable(t, parent)
	requireReachable(t, grandchild)
	if err := owner.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	if err := owner.Wait(); err == nil {
		t.Fatal("owner did not terminate abnormally")
	}
	requireStopped(t, parent)
	requireStopped(t, grandchild)
	t.Logf("verified owner death tree cleanup on %s", runtime.GOOS)
}

func ExampleConfig() {
	configuration := Config{Path: "litradar", OutputLimit: 4096}
	fmt.Println(configuration.OutputLimit)
	// Output: 4096
}
