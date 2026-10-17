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

// TestIndexParserRejectsInvalidExecutionOptions checks actionable failures for invalid input.
func TestIndexParserRejectsInvalidExecutionOptions(t *testing.T) {
	for _, item := range []struct {
		name    string
		values  []string
		message string
	}{
		{"zero workers", []string{"--workers", "0"}, "--workers must be at least 1"},
		{"invalid timeout", []string{"--timeout", "invalid"}, "invalid syntax"},
	} {
		t.Run(item.name, func(t *testing.T) {
			args := arguments(item.values)
			if _, _, err := parseIndex(&args); err == nil || !strings.Contains(err.Error(), item.message) {
				t.Fatalf("expected %q, got %v", item.message, err)
			}
		})
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
