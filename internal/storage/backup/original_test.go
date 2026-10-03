package backup

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

func TestGoBackupVerifiedAndRestoredByOriginalRust(t *testing.T) {
	oracle := os.Getenv("LITRADAR_RUST_STORAGE_BACKUP_ORACLE")
	if oracle == "" {
		t.Skip("explicit interoperability runner supplies the original Rust oracle")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	source := fixtureConfig(t)
	options := optionsFor(t, source)
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	target := fixtureConfig(t)
	write(t, filepath.Join(target.MetaDir, "nested", "custom.csv"), []byte("obsolete target metadata\n"))
	write(t, filepath.Join(target.MetaDir, "target-only.csv"), []byte("must disappear"))
	write(t, filepath.Join(target.ProjectRoot, "data", "push_state", "run.json"), []byte(`{"target":"obsolete"}`))
	write(t, filepath.Join(target.ProjectRoot, "data", "push_state", "target-only.json"), []byte("{}"))
	requests := []map[string]string{{"operation": "verify", "root": options.OutputDir}, {"operation": "restore", "root": target.ProjectRoot, "backup": options.OutputDir}}
	input := bytes.Buffer{}
	for _, request := range requests {
		raw, err := json.Marshal(request)
		if err != nil {
			t.Fatal(err)
		}
		input.Write(raw)
		input.WriteByte('\n')
	}
	command := exec.CommandContext(ctx, oracle)
	command.Stdin = &input
	diagnostics := bytes.Buffer{}
	command.Stderr = &diagnostics
	output, err := command.Output()
	if err != nil {
		t.Fatalf("original Rust: %v %s", err, diagnostics.String())
	}
	lines := bytes.Split(bytes.TrimSpace(output), []byte("\n"))
	if len(lines) != 2 {
		t.Fatalf("unexpected response %s", output)
	}
	for index, line := range lines {
		var result struct {
			Error  *string
			Output json.RawMessage
		}
		if err := json.Unmarshal(line, &result); err != nil {
			t.Fatal(err)
		}
		if result.Error != nil {
			t.Fatal(*result.Error)
		}
		if index == 0 {
			var verified Manifest
			if err := json.Unmarshal(result.Output, &verified); err != nil || !reflect.DeepEqual(manifest, verified) {
				t.Fatalf("%+v %v", verified, err)
			}
		}
	}
	for _, component := range manifest.Components {
		destination := target.AuthDbPath
		if component.Kind != "auth_database" {
			destination = filepath.Join(target.ProjectRoot, "data", filepath.FromSlash(component.Path))
		}
		if err := validateFile(ctx, component, destination); err != nil {
			t.Fatal(err)
		}
	}
	for _, filename := range []string{filepath.Join(target.MetaDir, "target-only.csv"), filepath.Join(target.ProjectRoot, "data", "push_state", "target-only.json")} {
		if _, err := os.Stat(filename); !os.IsNotExist(err) {
			t.Fatalf("selected group retained obsolete file %s: %v", filename, err)
		}
	}
}

func TestOriginalRustManifestDecoding(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "migration", "storage", "backup-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Input  string
			Output struct {
				Valid    bool
				Manifest json.RawMessage
			}
		}
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		t.Run("manifest", func(t *testing.T) {
			actual, err := ParseManifest([]byte(scenario.Input))
			if !scenario.Output.Valid {
				if err == nil {
					t.Fatalf("accepted %s", scenario.Input)
				}
				return
			}
			if err != nil {
				t.Fatalf("%s: %v", scenario.Input, err)
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			json.Unmarshal(encoded, &got)
			json.Unmarshal(scenario.Output.Manifest, &want)
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("got %s want %s", encoded, scenario.Output.Manifest)
			}
		})
	}
}

func TestVerifyOriginalRustBackupAndRestore(t *testing.T) {
	source := filepath.Join("..", "..", "..", "tests", "migration", "storage", "fixtures", "backup-v2")
	files, err := snapshot(source)
	if err != nil {
		t.Fatal(err)
	}
	destination := t.TempDir()
	for _, file := range files {
		if err := copyFile(filepath.Join(source, filepath.FromSlash(file.Path)), filepath.Join(destination, filepath.FromSlash(file.Path))); err != nil {
			t.Fatal(err)
		}
	}
	manifest, err := Verify(context.Background(), destination)
	if err != nil || manifest.Version != 2 || len(manifest.Components) != 4 {
		t.Fatalf("%+v %v", manifest, err)
	}
	target := fixtureConfig(t)
	if _, err := restore(context.Background(), RestoreOptions{target, target.AuthDbPath, destination}, 1000, nil, nil); err != nil {
		t.Fatal(err)
	}
}

func TestComponentPathExtensionUsesRustDotfileRules(t *testing.T) {
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
