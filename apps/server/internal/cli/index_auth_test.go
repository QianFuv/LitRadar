package cli

import (
	"path/filepath"
	"testing"
)

// TestIndexInvocationRetainsNotificationAuthDatabase keeps the resolved store in notification configuration.
func TestIndexInvocationRetainsNotificationAuthDatabase(t *testing.T) {
	root := t.TempDir()
	for _, custom := range []string{"", filepath.Join(root, "private auth", "accounts.sqlite")} {
		t.Run(custom, func(t *testing.T) {
			args := arguments{"--project-root", root, "--update", "--notify"}
			expected := filepath.Join(root, "data", "auth.sqlite")
			if custom != "" {
				args = append(args, "--auth-db", custom)
				expected = custom
			}
			invocation, err := parseIndexInvocation(&args)
			if err != nil {
				t.Fatal(err)
			}
			if invocation.storage.AuthDbPath != expected || invocation.options.AuthDbPath != expected {
				t.Fatalf("auth database mismatch: storage=%q notification=%q expected=%q", invocation.storage.AuthDbPath, invocation.options.AuthDbPath, expected)
			}
		})
	}
}
