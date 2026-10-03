package process

import (
	"context"
	"errors"
	"os/exec"
	"testing"
	"time"

	"golang.org/x/sys/windows"
)

func TestFailedSuspendedStartActuallyExits(t *testing.T) {
	for _, stage := range []string{"before_assignment", "before_resume"} {
		t.Run(stage, func(t *testing.T) {
			var handle windows.Handle
			_, err := startWithHook(context.Background(), fixtureConfig("parent", t.TempDir()), func(current string, command *exec.Cmd) error {
				if current != stage {
					return nil
				}
				var err error
				handle, err = windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(command.Process.Pid))
				if err != nil {
					t.Fatal(err)
				}
				return errors.New("injected failure")
			})
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
