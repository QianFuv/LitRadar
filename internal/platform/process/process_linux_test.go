package process

import (
	"io"
	"os"
	"path/filepath"
	"syscall"
	"testing"
)

func TestParentLossBeforeGuardStopsWholeGroup(t *testing.T) {
	directory := t.TempDir()
	owner := fixtureCommand("owner-late-guard", directory)
	owner.Stdout, owner.Stderr = io.Discard, io.Discard
	if err := owner.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { owner.Process.Kill(); owner.Wait() })
	worker := awaitFixture(t, directory, "late-guard")
	grandchild := awaitFixture(t, directory, "grandchild")
	t.Cleanup(func() { syscall.Kill(-worker.Pid, syscall.SIGKILL) })
	requireReachable(t, worker)
	requireReachable(t, grandchild)
	if err := owner.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	owner.Wait()
	if err := os.WriteFile(filepath.Join(directory, "release-guard"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	requireStopped(t, worker)
	requireStopped(t, grandchild)
	if _, err := os.Stat(filepath.Join(directory, "work-started")); !os.IsNotExist(err) {
		t.Fatal("work began after pre-start parent death")
	}
}

func TestParentGuardRejectsInvalidOwnershipBeforeWatching(t *testing.T) {
	for _, value := range []string{"", "bad", "-1", "0", "2147483648"} {
		t.Run(value, func(t *testing.T) {
			t.Setenv(ParentEnvironment, value)
			if stop, err := StartParentGuard(); err == nil {
				stop()
				t.Fatal("invalid ownership accepted")
			}
		})
	}
}
