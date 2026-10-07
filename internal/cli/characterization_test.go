package cli

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// TestParentMarkerFailureRetainsConsumedArgumentBoundary checks mutation before uniqueness and grammar errors.
func TestParentMarkerFailureRetainsConsumedArgumentBoundary(t *testing.T) {
	for _, item := range []struct{ values, remaining []string }{
		{[]string{"notify", "--litradar-parent-run-id", "bad/id", "--dry-run"}, []string{"notify", "--dry-run"}},
		{[]string{"--litradar-parent-run-id", "first", "--litradar-parent-run-id", "second"}, []string{"--litradar-parent-run-id", "second"}},
	} {
		args := arguments(append([]string{}, item.values...))
		if _, err := parentRunId(&args); err == nil || !reflect.DeepEqual([]string(args), item.remaining) {
			t.Fatal(args, err)
		}
	}
}

// TestIndexParserFailureRetainsPartialValuesAndUnconsumedModes checks competing option failures.
func TestIndexParserFailureRetainsPartialValuesAndUnconsumedModes(t *testing.T) {
	args := arguments{"--file", "catalog.csv", "--workers", "0", "--issue-batch", "4", "--update"}
	configuration, explicit, err := parseIndex(&args)
	expectedFile := "catalog.csv"
	expectedWorkers := uint64(0)
	expected := []any{&expectedFile, &expectedWorkers, uint64(0), uint64(0), false}
	actual := []any{configuration.File, configuration.WorkerCount, configuration.IssueBatchSize, configuration.TimeoutSeconds, configuration.ShouldUpdate}
	if err == nil || err.Error() != "--workers must be at least 1" || explicit || !reflect.DeepEqual(actual, expected) {
		t.Fatal(configuration, explicit, err)
	}
	if !reflect.DeepEqual([]string(args), []string{"--issue-batch", "4", "--update"}) {
		t.Fatal(args)
	}
	assertIndexTimeoutFailureState(t)
}

// TestServeDevelopmentFailureReturnsPopulatedConfiguration checks final validation's distinct return contract.
func TestServeDevelopmentFailureReturnsPopulatedConfiguration(t *testing.T) {
	configuration, err := parseServeWithBundle([]string{"--project-root", "fixture", "--host", "0.0.0.0", "--port", "9001", "--secret-key-file", "key", "--development"}, "canonical", "immutable/meta")
	expected := []any{"0.0.0.0", uint16(9001), "key", "canonical", "immutable/meta", true}
	actual := []any{configuration.Host, configuration.Port, configuration.SecretKeyFile, configuration.Executable, configuration.BundledMetaDir, configuration.IsDevelopment}
	if err == nil || !strings.Contains(err.Error(), "Development mode requires") || !reflect.DeepEqual(actual, expected) {
		t.Fatal(configuration, err)
	}
}

// TestDeliveryTargetInferenceRetainsDuplicateLastValueAndDirectoryAdmission checks exact changes-file grammar.
func TestDeliveryTargetInferenceRetainsDuplicateLastValueAndDirectoryAdmission(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	directory := filepath.Join(configuration.IndexDir, "visible.sqlite")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	changes := filepath.Join(configuration.ProjectRoot, "changes.json")
	if err := os.WriteFile(changes, []byte(`{"db_name":"missing","db_name":"visible"}`), 0600); err != nil {
		t.Fatal(err)
	}
	expected := []deliveryTarget{{directory, "visible.sqlite"}}
	for _, selected := range []*string{nil, &changes} {
		actual, err := deliveryTargets(configuration, nil, nil, selected)
		if err != nil || !reflect.DeepEqual(actual, expected) {
			t.Fatal(actual, err)
		}
	}
}

// assertIndexTimeoutFailureState checks explicit legacy state before an invalid timeout leaves later modes untouched.
func assertIndexTimeoutFailureState(t *testing.T) {
	t.Helper()
	args := arguments{"--issue-batch", "4", "--timeout", "invalid", "--processes", "0", "--notify"}
	configuration, explicit, err := parseIndex(&args)
	if err == nil || !explicit || configuration.IssueBatchSize != 4 || configuration.TimeoutSeconds != 20 {
		t.Fatal(configuration, explicit, err)
	}
	if !reflect.DeepEqual([]string(args), []string{"--processes", "0", "--notify"}) {
		t.Fatal(args)
	}
}
