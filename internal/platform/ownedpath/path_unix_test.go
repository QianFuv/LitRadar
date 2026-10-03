//go:build !windows

package ownedpath

import (
	"os"
	"testing"
)

func createDirectoryLink(t *testing.T, link, target string) {
	t.Helper()
	if err := os.Symlink(target, link); err != nil {
		t.Fatal(err)
	}
}
