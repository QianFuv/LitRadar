//go:build !windows

package ownedpath

import (
	"os"
	"syscall"
)

func isLink(metadata os.FileInfo) bool { return metadata.Mode()&os.ModeSymlink != 0 }

func hasMultipleLinks(metadata os.FileInfo) bool {
	attributes, ok := metadata.Sys().(*syscall.Stat_t)
	return !ok || attributes.Nlink != 1
}
