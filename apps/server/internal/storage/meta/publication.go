package meta

import (
	"os"
	"path/filepath"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

func publishFile(source, target string) error {
	root, name, err := rootfile.Parent(target)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	relative, err := filepath.Rel(root.Name(), source)
	if err != nil {
		return err
	}
	if err := root.Link(relative, name); err != nil {
		return err
	}
	_ = root.Remove(relative)
	return nil
}

func removeFile(filename string) error {
	root, name, err := rootfile.Parent(filename)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	info, err := root.Lstat(name)
	if err != nil {
		return err
	}
	if info.IsDir() {
		return &os.PathError{Op: "remove", Path: filename, Err: os.ErrInvalid}
	}
	return root.Remove(name)
}
