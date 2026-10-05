package index

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

func TestSharedGroupRejectsBeforeReadingWorkerRequest(t *testing.T) {
	const marker = "LITRADAR_TEST_SHARED_WORKER_GROUP"
	if os.Getenv(marker) == "1" {
		group, err := syscall.Getpgid(0)
		if err != nil || group == os.Getpid() {
			t.Fatal("fixture does not share group", group, err)
		}
		err = RunWorkerRequestFile(context.Background(), filepath.Join(t.TempDir(), "missing-request.json"))
		if err == nil || err.Error() != "worker parent ownership is invalid" {
			t.Fatal("request access preceded ownership validation", err)
		}
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	command := exec.CommandContext(ctx, executable, "-test.run=^TestSharedGroupRejectsBeforeReadingWorkerRequest$")
	command.Env = append(workerEnvironment(), marker+"=1")
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("shared group: %v\n%s", err, output)
	}
}
