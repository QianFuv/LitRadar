package backup

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"syscall"
	"testing"
)

func TestMetadataBackupRejectsLinksAndSpecialFiles(t *testing.T) {
	for _, stage := range []string{"create", "verify"} {
		for _, kind := range []string{"symlink", "socket", "fifo"} {
			t.Run(stage+"/"+kind, func(t *testing.T) {
				configuration := fixtureConfig(t)
				options := optionsFor(t, configuration)
				options.IncludeIndexDatabases = false
				options.IncludePushState = false
				directory := configuration.MetaDir
				if stage == "verify" {
					if _, err := Create(context.Background(), options); err != nil {
						t.Fatal(err)
					}
					directory = filepath.Join(options.OutputDir, "meta")
				}
				target := filepath.Join(directory, "nested", "custom.csv")
				if err := os.Remove(target); err != nil {
					t.Fatal(err)
				}
				switch kind {
				case "symlink":
					if err := os.Symlink(configuration.AuthDbPath, target); err != nil {
						t.Fatal(err)
					}
				case "socket":
					listener, err := net.Listen("unix", target)
					if err != nil {
						t.Fatal(err)
					}
					t.Cleanup(func() { listener.Close() })
				case "fifo":
					if err := syscall.Mkfifo(target, 0600); err != nil {
						t.Fatal(err)
					}
				}
				if stage == "create" {
					_, err := Create(context.Background(), options)
					requireFailure(t, err, "integrity")
					if _, err := os.Lstat(options.OutputDir); !os.IsNotExist(err) {
						t.Fatal("invalid metadata published backup", err)
					}
				} else {
					_, err := Verify(context.Background(), options.OutputDir)
					requireFailure(t, err, "integrity")
				}
			})
		}
	}
}
