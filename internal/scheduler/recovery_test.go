package scheduler

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
)

func TestSchedulerRealCrashRecovery(t *testing.T) {
	for _, boundary := range []string{"claimed", "running", "finished"} {
		t.Run(boundary, func(t *testing.T) {
			repository, filename := schedulerFixture(t)
			task := createTask(t, repository)
			if _, err := repository.Enqueue(context.Background(), task, []int64{60}); err != nil {
				t.Fatal(err)
			}
			t.Setenv("LITRADAR_SCHEDULER_TEST_ROLE", "claim-crash")
			t.Setenv("LITRADAR_SCHEDULER_TEST_DIRECTORY", filepath.Dir(filename))
			t.Setenv("LITRADAR_SCHEDULER_TEST_BOUNDARY", boundary)
			executable, err := os.Executable()
			if err != nil {
				t.Fatal(err)
			}
			command := exec.Command(executable)
			if err := command.Start(); err != nil {
				t.Fatal(err)
			}
			didWait := false
			defer func() {
				if !didWait {
					command.Process.Kill()
					command.Wait()
				}
			}()
			hasReached := false
			for deadline := time.Now().Add(8 * time.Second); time.Now().Before(deadline); {
				if data, err := os.ReadFile(filepath.Join(filepath.Dir(filename), "barrier")); err == nil && string(data) == boundary {
					hasReached = true
					break
				}
				time.Sleep(10 * time.Millisecond)
			}
			if !hasReached {
				t.Fatal("child did not reach durable barrier")
			}
			if err := command.Process.Kill(); err != nil {
				t.Fatal(err)
			}
			err = command.Wait()
			didWait = true
			if err == nil {
				t.Fatal("child exited normally instead of being killed")
			}
			claims, err := repository.ClaimReady(context.Background(), "restarted", 191, 90, 1)
			if err != nil {
				t.Fatal(err)
			}
			expected := domain.Success
			if boundary == "claimed" {
				expected = domain.Claimed
				if len(claims) != 1 || claims[0].WorkerId != "restarted" {
					t.Fatal("unstarted work was not recovered")
				}
			} else {
				if len(claims) != 0 {
					t.Fatal("ambiguous or complete work was replayed")
				}
				if boundary == "running" {
					expected = domain.Unknown
				}
				if data, err := os.ReadFile(filepath.Join(filepath.Dir(filename), "effect")); err != nil || string(data) != "executed" {
					t.Fatal("effect evidence absent")
				}
			}
			status, err := repository.Status(context.Background(), 191, 90, 10)
			if err != nil || len(status.RecentRuns) != 1 || status.RecentRuns[0].Status != expected {
				t.Fatalf("%#v %v", status, err)
			}
		})
	}
}
