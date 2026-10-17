package index

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// TestNotificationProcessPreservesAuthDatabase checks actual child arguments for custom and default stores.
func TestNotificationProcessPreservesAuthDatabase(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	for _, database := range []string{"", filepath.Join(t.TempDir(), "private auth", "accounts.sqlite")} {
		t.Run(database, func(t *testing.T) {
			t.Setenv("LITRADAR_NOTIFY_CHILD_FIXTURE", "completed")
			t.Setenv("LITRADAR_NOTIFY_EXPECTED_AUTH_DB", database)
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			config := NotifyProcessConfig{Executable: executable, SecretKeyFile: "private-key-path", ProjectRoot: "project-root", AuthDbPath: database}
			observed, err := RunNotifyProcess(ctx, config, "catalog.sqlite", "manifest-path", "current-attempt")
			if err != nil || observed.Status != storage.NotifyCompleted || observed.ExitCode == nil || *observed.ExitCode != 0 {
				t.Fatalf("notification did not receive auth database %q: observed=%+v err=%v", database, observed, err)
			}
		})
	}
}
