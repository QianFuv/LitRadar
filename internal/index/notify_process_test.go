package index

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"testing"
	"time"

	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func TestMain(tests *testing.M) {
	if len(os.Args) > 1 && os.Args[1] == "--index-crash-fixture" {
		if err := runCrashFixture(os.Getenv("LITRADAR_INDEX_CRASH_ROOT"), os.Getenv("LITRADAR_INDEX_CRASH_POINT")); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(95)
		}
		os.Exit(0)
	}
	if mode := os.Getenv("LITRADAR_NOTIFY_CHILD_FIXTURE"); mode != "" && len(os.Args) > 1 && os.Args[1] == "notify" {
		runNotifyChildFixture(mode)
		os.Exit(0)
	}
	os.Exit(tests.Run())
}

func runNotifyChildFixture(mode string) {
	options := map[string]string{}
	isDryRun := false
	for position := 2; position < len(os.Args); position++ {
		argument := os.Args[position]
		if argument == "--dry-run" {
			isDryRun = true
			continue
		}
		if argument == "--internal-handoff-json" {
			options[argument] = "true"
			continue
		}
		if position+1 >= len(os.Args) {
			os.Exit(90)
		}
		position++
		options[argument] = os.Args[position]
	}
	if options["--secret-key-file"] != "private-key-path" || options["--changes-file"] != "manifest-path" || options["--project-root"] != "project-root" || options["--internal-handoff-json"] != "true" {
		os.Exit(91)
	}
	if mode == "wait" {
		time.Sleep(time.Minute)
		return
	}
	if _, err := io.Copy(io.Discard, os.Stdin); err != nil {
		os.Exit(92)
	}
	payload := notifyPayload{ProtocolVersion: 1, AttemptId: options["--attempt-id"], Workflow: "notify", Mode: "execute", Status: "completed", DbName: options["--db"]}
	if isDryRun {
		payload.Mode = "dry_run"
	}
	if mode == "stale" {
		payload.AttemptId = "previous-attempt"
	}
	if mode == "failed" {
		payload.Status = "failed"
	}
	if mode == "oversize" {
		fmt.Fprint(os.Stdout, strings.Repeat(" ", 1024*1024))
	}
	if err := json.NewEncoder(os.Stdout).Encode(payload); err != nil {
		os.Exit(93)
	}
	if mode == "failed" || mode == "inconsistent" {
		os.Exit(1)
	}
}

func TestActualNotificationProcessDrainsAndClassifies(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	for _, item := range []struct {
		mode     string
		isDryRun bool
		status   storage.NotifyStatus
		exit     int32
	}{
		{"completed", false, storage.NotifyCompleted, 0},
		{"completed", true, storage.NotifyCompleted, 0},
		{"failed", false, storage.NotifyFailed, 1},
		{"stale", false, storage.NotifyUnknown, 0},
		{"inconsistent", false, storage.NotifyUnknown, 1},
		{"oversize", false, storage.NotifyUnknown, 0},
	} {
		t.Run(fmt.Sprintf("%s/dry=%t", item.mode, item.isDryRun), func(t *testing.T) {
			t.Setenv("LITRADAR_NOTIFY_CHILD_FIXTURE", item.mode)
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			observed, err := RunNotifyProcess(ctx, NotifyProcessConfig{executable, "private-key-path", "project-root", item.isDryRun}, "catalog.sqlite", "manifest-path", "current-attempt")
			if err != nil || observed.Status != item.status || observed.ExitCode == nil || *observed.ExitCode != item.exit {
				t.Fatalf("observed=%+v err=%v", observed, err)
			}
		})
	}
}

func TestActualNotificationCancellationAndSpawnFailure(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("LITRADAR_NOTIFY_CHILD_FIXTURE", "wait")
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	observed, err := RunNotifyProcess(ctx, NotifyProcessConfig{executable, "private-key-path", "project-root", false}, "catalog.sqlite", "manifest-path", "current-attempt")
	if err != nil || observed.Status != storage.NotifyUnknown || ctx.Err() == nil {
		t.Fatalf("observed=%+v err=%v context=%v", observed, err, ctx.Err())
	}
	if _, err := RunNotifyProcess(context.Background(), NotifyProcessConfig{Executable: "missing-notify-executable"}, "db", "manifest", "attempt"); err == nil {
		t.Fatal("spawn failure was swallowed")
	}
}
