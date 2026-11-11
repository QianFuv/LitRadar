package index

import (
	"os"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

func removeWorkerFile(path string) error {
	root, name, err := rootfile.Parent(path)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	info, err := root.Lstat(name)
	if err != nil {
		return err
	}
	if info.IsDir() {
		return &os.PathError{Op: "remove", Path: path, Err: os.ErrInvalid}
	}
	return root.Remove(name)
}
