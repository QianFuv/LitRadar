package ownedpath

import (
	"os"
	"os/exec"
	"syscall"
	"testing"
)

func createDirectoryLink(t *testing.T, link, target string) {
	t.Helper()
	command := exec.Command("cmd", "/c", "mklink", "/J", link, target)
	command.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x08000000}
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("junction fixture: %s %v", output, err)
	}
	t.Cleanup(func() {
		if err := os.Remove(link); err != nil {
			t.Error(err)
		}
	})
}
