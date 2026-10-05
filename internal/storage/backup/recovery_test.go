package backup

import (
	"context"

	"os"

	"path/filepath"

	"testing"
)

func TestComponentPathExtensionHandlesDotfiles(t *testing.T) {
	for _, name := range []string{"meta/.key", "meta/.pem", "meta/file.Key"} {
		if keyFile(name) {
			t.Fatalf("unexpected key classification %q", name)
		}
	}
	for _, name := range []string{"meta/file.KEY", "meta/file.PeM"} {
		if !keyFile(name) {
			t.Fatalf("missed key %q", name)
		}
	}
	version := int64(9)
	if err := validateLayout(Component{Kind: "index_database", Path: "index/.sqlite", SchemaVersion: &version}, Selection{IndexDatabases: true}); err == nil {
		t.Fatal("dotfile has no extension in original path semantics")
	}
}

func TestPartialApplyFailureRestoresPreviouslyReplacedFiles(t *testing.T) {
	root := t.TempDir()
	first := filepath.Join(root, "first")
	second := filepath.Join(root, "second")
	write(t, first, []byte("original-one"))
	write(t, second, []byte("original-two"))
	stage := filepath.Join(root, "staged")
	write(t, stage, []byte("new"))
	items := []replacement{{target: first, staged: stage, rollback: filepath.Join(root, "rollback-one")}, {target: second, staged: filepath.Join(root, "missing-stage"), rollback: filepath.Join(root, "rollback-two")}}
	if err := apply(items); err == nil {
		t.Fatal("expected rename failure")
	}
	for filename, want := range map[string]string{first: "original-one", second: "original-two"} {
		actual, err := os.ReadFile(filename)
		if err != nil || string(actual) != want {
			t.Fatalf("%s %q %v", filename, actual, err)
		}
	}
}

func TestRollbackFailurePreservesRecoveryWorkspaces(t *testing.T) {
	ctx := context.Background()
	source := fixtureConfig(t)
	options := optionsFor(t, source)
	if _, err := Create(ctx, options); err != nil {
		t.Fatal(err)
	}
	target := fixtureConfig(t)
	var recovery string
	_, err := restore(ctx, RestoreOptions{target, target.AuthDbPath, options.OutputDir}, 1000, nil, func() error {
		matches, err := filepath.Glob(filepath.Join(target.ProjectRoot, "data", ".litradar-data-restore-*"))
		if err != nil {
			return err
		}
		recovery = matches[0]
		if err := os.Rename(filepath.Join(recovery, "rollback-meta"), filepath.Join(recovery, "saved-original-meta")); err != nil {
			return err
		}
		return failure("integrity", "injected validation failure")
	})
	requireFailure(t, err, "rollback")
	if !fileExists(filepath.Join(recovery, "saved-original-meta", "nested", "custom.csv")) {
		t.Fatal("original recovery copy was discarded")
	}
	authWorkspaces, err := filepath.Glob(filepath.Join(target.ProjectRoot, "data", ".litradar-auth-restore-*"))
	if err != nil || len(authWorkspaces) != 1 {
		t.Fatalf("auth recovery=%v %v", authWorkspaces, err)
	}
}
