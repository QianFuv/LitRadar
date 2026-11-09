package process

import (
	"io"
	"os"
	"syscall"
	"testing"
	"time"
)

func TestParentTerminationAndStoppedGuardPreserveOwnership(t *testing.T) {
	for _, scenario := range []struct {
		name, ownerMode, workerMode string
		signal                      syscall.Signal
		shouldSurvive               bool
	}{
		{"sigterm", "owner", "parent", syscall.SIGTERM, false},
		{"stopped", "owner-stopped-guard", "stopped-guard", syscall.SIGKILL, true},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			directory := t.TempDir()
			owner := fixtureCommand(scenario.ownerMode, directory)
			owner.Stdout, owner.Stderr = io.Discard, io.Discard
			if err := owner.Start(); err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { owner.Process.Kill(); owner.Wait() })
			worker := awaitFixture(t, directory, scenario.workerMode)
			descendant := awaitFixture(t, directory, "grandchild")
			group, err := syscall.Getpgid(worker.Pid)
			if err != nil || group != worker.Pid {
				t.Fatal("worker group", group, err)
			}
			t.Cleanup(func() { syscall.Kill(-worker.Pid, syscall.SIGKILL) })
			requireReachable(t, worker)
			requireReachable(t, descendant)
			if err := owner.Process.Signal(scenario.signal); err != nil {
				t.Fatal(err)
			}
			if err := owner.Wait(); err == nil {
				t.Fatal("owner did not terminate")
			}
			if scenario.shouldSurvive {
				time.Sleep(350 * time.Millisecond)
				requireReachable(t, worker)
				requireReachable(t, descendant)
			} else {
				requireStopped(t, worker)
				requireStopped(t, descendant)
			}
		})
	}
}

func TestParentGuardRejectsMissingOwnership(t *testing.T) {
	t.Setenv(ParentEnvironment, "temporary")
	if err := os.Unsetenv(ParentEnvironment); err != nil {
		t.Fatal(err)
	}
	if stop, err := StartParentGuard(); err == nil {
		stop()
		t.Fatal("missing ownership accepted")
	}
}
