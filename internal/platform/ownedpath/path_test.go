package ownedpath

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// TestRejectLinkedAncestryWithoutTouchingTarget retains link, traversal and owned-file boundaries.
func TestRejectLinkedAncestryWithoutTouchingTarget(t *testing.T) {
	root := t.TempDir()
	assertLinkedAncestryUntouched(t, root)
	ordinary := filepath.Join(root, "ordinary", "nested")
	if _, err := Prepare(ordinary); err != nil {
		t.Fatal(err)
	}
	if _, err := Prepare(ordinary + string(os.PathSeparator) + ".." + string(os.PathSeparator) + "escape"); err == nil {
		t.Fatal("parent component accepted")
	}
	file := filepath.Join(ordinary, "file")
	if err := os.WriteFile(file, []byte("owned"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := Validate(file, false); err != nil {
		t.Fatal(err)
	}
	if err := os.Link(file, filepath.Join(ordinary, "hardlink")); err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" && Validate(file, false) == nil {
		t.Fatal("multiply linked workset accepted")
	}
}

// assertLinkedAncestryUntouched proves a rejected ancestor leaves the destination and sentinel unchanged.
func assertLinkedAncestryUntouched(t *testing.T, root string) {
	t.Helper()
	outside := filepath.Join(root, "outside")
	if err := os.Mkdir(outside, 0755); err != nil {
		t.Fatal(err)
	}
	sentinel := filepath.Join(outside, "sentinel")
	if err := os.WriteFile(sentinel, []byte("unchanged"), 0600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	createDirectoryLink(t, link, outside)
	if _, err := Prepare(filepath.Join(link, "must-not-create")); err == nil {
		t.Fatal("linked ancestor accepted")
	}
	entries, err := os.ReadDir(outside)
	if err != nil || len(entries) != 1 {
		t.Fatalf("destination changed: %v %v", entries, err)
	}
	body, err := os.ReadFile(sentinel)
	if err != nil || !bytes.Equal(body, []byte("unchanged")) {
		t.Fatal("sentinel changed", err)
	}
}
