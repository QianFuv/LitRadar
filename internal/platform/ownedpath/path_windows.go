package ownedpath

import (
	"os"
	"syscall"
)

func isLink(metadata os.FileInfo) bool {
	attributes, ok := metadata.Sys().(*syscall.Win32FileAttributeData)
	return !ok || attributes.FileAttributes&syscall.FILE_ATTRIBUTE_REPARSE_POINT != 0
}

func hasMultipleLinks(os.FileInfo) bool { return false }
