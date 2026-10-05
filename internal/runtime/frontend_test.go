package runtime

import (
	"bytes"

	"os"
	"path/filepath"

	"testing"
)

func runtimeConfiguration(t *testing.T) Config {
	t.Helper()
	root := t.TempDir()
	key := filepath.Join(root, "secret.key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	configuration, err := NewConfig(root, "127.0.0.1", 0, key)
	if err != nil {
		t.Fatal(err)
	}
	configuration.IsDevelopment = true
	return configuration
}
