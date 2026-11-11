package scholarly

import (
	"os"
	"syscall"
)

func isWorksetLink(metadata os.FileInfo) bool {
	return metadata.Sys().(*syscall.Win32FileAttributeData).FileAttributes&syscall.FILE_ATTRIBUTE_REPARSE_POINT != 0
}

func hasWorksetHardLinks(os.FileInfo) bool { return false }
