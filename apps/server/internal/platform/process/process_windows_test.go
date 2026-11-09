package process

import (
	"context"
	"errors"
	"os/exec"
	"testing"
	"time"

	"golang.org/x/sys/windows"
)

// TestTreeCompletionAtCancellationRechecksNativeState preserves completion and native errors at the deadline.
func TestTreeCompletionAtCancellationRechecksNativeState(t *testing.T) {
	for _, scenario := range []struct {
		name        string
		isComplete  bool
		nativeError error
		wantError   error
	}{
		{name: "completed", isComplete: true},
		{name: "still-pending", wantError: context.Canceled},
		{name: "native-error", nativeError: windows.ERROR_INVALID_HANDLE, wantError: windows.ERROR_INVALID_HANDLE},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			hasSampled := false
			err := waitForTreeEmpty(ctx, func() (bool, error) {
				if !hasSampled {
					hasSampled = true
					cancel()
					return false, nil
				}
				return scenario.isComplete, scenario.nativeError
			})
			if !errors.Is(err, scenario.wantError) {
				t.Fatalf("native completion at cancellation: got %v, want %v", err, scenario.wantError)
			}
		})
	}
}

func TestFailedSuspendedStartActuallyExits(t *testing.T) {
	for _, stage := range []string{"before_assignment", "before_resume"} {
		t.Run(stage, func(t *testing.T) {
			var handle windows.Handle
			var probeError error
			_, err := startWithHook(context.Background(), fixtureConfig("parent", t.TempDir()), func(current string, command *exec.Cmd) error {
				if current != stage {
					return nil
				}
				handle, probeError = windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(command.Process.Pid))
				if probeError != nil {
					return probeError
				}
				return errors.New("injected failure")
			})
			if probeError != nil {
				t.Fatal(probeError)
			}
			if err == nil || handle == 0 {
				t.Fatal("failure not injected")
			}
			defer windows.CloseHandle(handle)
			state, err := windows.WaitForSingleObject(handle, 0)
			if err != nil || state != windows.WAIT_OBJECT_0 {
				t.Fatalf("suspended process leaked: %d %v", state, err)
			}
		})
	}
}

func TestDetachedWindowsExitDiagnostics(t *testing.T) {
	directory := t.TempDir()
	child, err := Start(context.Background(), fixtureConfig("detached-pipes", directory))
	if err != nil {
		t.Fatal(err)
	}
	defer child.Close()
	grandchild := awaitFixture(t, directory, "grandchild")
	handle, err := windows.OpenProcess(windows.SYNCHRONIZE|windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(grandchild.Pid))
	if err != nil {
		t.Fatal(err)
	}
	defer windows.CloseHandle(handle)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := child.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	if err := child.Close(); err != nil {
		t.Fatal(err)
	}
	state, err := windows.WaitForSingleObject(handle, 0)
	var code uint32
	windows.GetExitCodeProcess(handle, &code)
	t.Logf("descendant pid=%d handle-state=%d exit=%d error=%v", grandchild.Pid, state, code, err)
	if state != windows.WAIT_OBJECT_0 || err != nil {
		t.Fatal("process not terminated at Close return")
	}
	requireStopped(t, grandchild)
}
