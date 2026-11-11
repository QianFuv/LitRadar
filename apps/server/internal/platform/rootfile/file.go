// Package rootfile opens file parents for operations confined by os.Root.
package rootfile

import (
	"crypto/rand"
	"os"
	"path/filepath"
)

// Parent opens the containing directory and returns the leaf name within it.
// Callers with a larger managed tree should open that root once and use relative paths.
func Parent(filename string) (*os.Root, string, error) {
	root, err := os.OpenRoot(filepath.Dir(filename))
	return root, filepath.Base(filename), err
}

// CreateTemp creates a random exclusive file inside an already opened root.
// os.Root has no CreateTemp method; the random name is never resolved outside root.
func CreateTemp(root *os.Root, prefix string, mode os.FileMode) (*os.File, error) {
	return root.OpenFile(prefix+rand.Text(), os.O_RDWR|os.O_CREATE|os.O_EXCL, mode)
}

// MkdirTemp creates an exclusive random directory inside an already opened root.
func MkdirTemp(root *os.Root, prefix string) (string, error) {
	name := prefix + rand.Text()
	if err := root.Mkdir(name, 0700); err != nil {
		return "", err
	}
	return filepath.Join(root.Name(), name), nil
}
