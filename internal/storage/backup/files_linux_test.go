package backup

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCopyPreservesSpecialRegularFilePermissions(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "source")
	target := filepath.Join(root, "copy")
	write(t, source, []byte("operator metadata"))
	mode := os.FileMode(0750) | os.ModeSetuid | os.ModeSetgid | os.ModeSticky
	if err := os.Chmod(source, mode); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(source)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode()&(os.ModeSetuid|os.ModeSetgid|os.ModeSticky) != mode&(os.ModeSetuid|os.ModeSetgid|os.ModeSticky) {
		t.Fatal("test filesystem did not retain requested source mode")
	}
	if err := copyFile(source, target); err != nil {
		t.Fatal(err)
	}
	copied, err := os.Stat(target)
	if err != nil || copied.Mode() != info.Mode() {
		t.Fatalf("source=%v copied=%v error=%v", info.Mode(), copied, err)
	}
}
