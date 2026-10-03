//go:build !windows

package scholarly

import (
	"os"
	"syscall"
)

func isWorksetLink(metadata os.FileInfo) bool { return metadata.Mode()&os.ModeSymlink != 0 }

func hasWorksetHardLinks(metadata os.FileInfo) bool {
	return metadata.Sys().(*syscall.Stat_t).Nlink != 1
}

func unlinkWorksetFile(path string) error { return syscall.Unlink(path) }
