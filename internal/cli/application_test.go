package cli

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func TestAdminSubcommandsPreserveArgumentBoundaries(t *testing.T) {
	for _, command := range []string{"secrets migrate", "secrets verify", "secrets rotate", "backup create", "backup verify", "backup restore", "index optimize-storage"} {
		args := arguments{command}
		if value, err := adminCommand(&args); err == nil && value == command {
			t.Fatal("one positional argument became a nested command", command)
		}
	}
}

func TestIndexAndDeliveryPreservePathComponents(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	selected := ".csv"
	if err := preflightIndex(context.Background(), configuration, &selected); err == nil {
		t.Fatal("extensionless dotfile admitted as CSV")
	}
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{".sqlite", "visible.sqlite"} {
		if err := os.WriteFile(filepath.Join(configuration.IndexDir, name), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	targets, err := deliveryTargets(configuration, nil, nil, nil)
	if err != nil || len(targets) != 1 || targets[0].name != "visible.sqlite" {
		t.Fatal("delivery selected extensionless dotfile", targets, err)
	}
	for input, expected := range map[string]string{"known/.": "known.sqlite", "known/./": "known.sqlite", "known/..": "known/...sqlite", " .sqlite ": ".sqlite", "": ".sqlite"} {
		if actual := normalizeDatabase(input); actual != expected {
			t.Errorf("%q: got %q, want %q", input, actual, expected)
		}
	}
}

func TestOpenapiCommandUsesBindingsWithoutDeploymentMutation(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "openapi.json")
	if err := Run(context.Background(), []string{"openapi", "--output", filename}, "unused", strings.NewReader(""), &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	actual, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	var document map[string]any
	if err := json.Unmarshal(actual, &document); err != nil {
		t.Fatal(err)
	}
	if document["openapi"] != "3.1.0" || document["paths"] == nil {
		t.Fatal("missing API document")
	}

	var output bytes.Buffer
	if err := Run(context.Background(), []string{"openapi"}, "unused", strings.NewReader(""), &output); err != nil || !bytes.Equal(output.Bytes(), actual) {
		t.Fatal("file/stdout document disagreement", err)
	}
	if err := Run(context.Background(), []string{"--litradar-parent-run-id", "bad/id", "--help"}, "unused", strings.NewReader(""), &bytes.Buffer{}); err == nil {
		t.Fatal("help bypassed internal marker validation")
	}
}

func TestCfpInvalidScopeAndBudgetDoNotCreateStorage(t *testing.T) {
	for _, tail := range [][]string{{"refresh"}, {"refresh", "--all", "--catalog-id", "fixture"}, {"refresh", "--all", "--source-timeout", "0"}, {"refresh", "--all", "--timeout", "3601"}, {"refresh", "--all", "--unexpected"}, {"refresh", "--all", "--capture-dir", "unused"}, {"refresh", "--all", "--resume-captures"}} {
		root := t.TempDir()
		args := append([]string{"cfp", "--project-root", root}, tail...)
		if err := Run(context.Background(), args, "unused", strings.NewReader(""), &bytes.Buffer{}); err == nil {
			t.Fatal("invalid CFP command accepted", tail)
		}
		if _, err := os.Stat(filepath.Join(root, "data", "auth.sqlite")); !os.IsNotExist(err) {
			t.Fatal("CFP validation touched storage", tail, err)
		}
	}
}

func TestIndexModesAndAliasPrecedence(t *testing.T) {
	for _, value := range []struct {
		args []string
		want string
	}{{[]string{"--update", "--full-rescan"}, "--update cannot be combined"}, {[]string{"--notify"}, "--notify requires"}, {[]string{"--acknowledge-unknown-notify"}, "requires --notify"}, {[]string{"--update", "--notify", "--no-resume", "--acknowledge-unknown-notify"}, "requires --resume"}, {[]string{"--workers", "33", "--notify"}, "worker_count must"}} {
		args := arguments(value.args)
		if _, _, err := parseIndex(&args); err == nil || !strings.Contains(err.Error(), value.want) {
			t.Fatal(value.args, err)
		}
	}
	args := arguments{"-w", "2", "--workers", "3", "--no-resume", "--resume", "--no-update", "--update"}
	configuration, _, err := parseIndex(&args)
	if err != nil || *configuration.WorkerCount != 3 || configuration.ShouldResume || configuration.ShouldUpdate || !reflect.DeepEqual([]string(args), []string{"-w", "2"}) {
		t.Fatal(configuration, args, err)
	}
}
