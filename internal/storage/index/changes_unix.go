//go:build !windows

package index

import (
	"golang.org/x/sys/unix"
	"os"
	"path/filepath"
)

func syncManifestDirectory(path string) error {
	directory, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
func removeManifestFile(path string) error { return unix.Unlink(path) }
