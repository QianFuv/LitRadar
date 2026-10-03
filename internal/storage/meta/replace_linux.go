package meta

import (
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

func publishFile(source, target string) error {
	return publishWithRename(source, target, func() error {
		return unix.Renameat2(unix.AT_FDCWD, source, unix.AT_FDCWD, target, unix.RENAME_NOREPLACE)
	})
}

func publishWithRename(source, target string, rename func() error) error {
	err := rename()
	if err == nil {
		return nil
	}
	if !errors.Is(err, unix.ENOSYS) && !errors.Is(err, unix.EINVAL) {
		return &os.LinkError{Op: "rename", Old: source, New: target, Err: err}
	}
	if err := os.Link(source, target); err != nil {
		return err
	}
	_ = unix.Unlink(source)
	return nil
}

func removeFile(filename string) error {
	if err := unix.Unlink(filename); err != nil {
		return &os.PathError{Op: "remove", Path: filename, Err: err}
	}
	return nil
}
