package meta

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"golang.org/x/sys/unix"
)

func TestRenameFallbackOnlyForUnsupportedAtomicOperation(t *testing.T) {
	for _, failure := range []error{unix.ENOSYS, unix.EINVAL, unix.EACCES, unix.ENOSPC, unix.EEXIST} {
		t.Run(failure.Error(), func(t *testing.T) {
			directory := t.TempDir()
			source, target := filepath.Join(directory, "source"), filepath.Join(directory, "target")
			writeFixture(t, source, []byte("new"))
			err := publishWithRename(source, target, func() error { return failure })
			if errors.Is(failure, unix.ENOSYS) || errors.Is(failure, unix.EINVAL) {
				if err != nil {
					t.Fatal(err)
				}
				data, err := os.ReadFile(target)
				if err != nil || string(data) != "new" {
					t.Fatal("fallback did not publish")
				}
			} else {
				if !errors.Is(err, failure) {
					t.Fatalf("original failure hidden: %v", err)
				}
				if _, err := os.Stat(target); !errors.Is(err, os.ErrNotExist) {
					t.Fatal("unexpected fallback publication")
				}
			}
		})
	}
}
