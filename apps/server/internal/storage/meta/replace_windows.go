package meta

import (
	"os"

	"golang.org/x/sys/windows"
)

func publishFile(source, target string) error {
	sourcePath, err := windows.UTF16PtrFromString(source)
	if err != nil {
		return err
	}
	targetPath, err := windows.UTF16PtrFromString(target)
	if err != nil {
		return err
	}
	if err := windows.MoveFileEx(sourcePath, targetPath, 0); err != nil {
		return &os.LinkError{Op: "rename", Old: source, New: target, Err: err}
	}
	return nil
}

func removeFile(filename string) error {
	if err := windows.Unlink(filename); err != nil {
		return &os.PathError{Op: "remove", Path: filename, Err: err}
	}
	return nil
}
