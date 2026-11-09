package cli

import (
	"math"
	"reflect"
	"strings"
	"testing"
)

func TestParentRunMarkerIsUniqueBoundedAndRemovedBeforeDispatch(t *testing.T) {
	args := arguments{"notify", "--litradar-parent-run-id", "run-123.safe", "--dry-run"}
	value, err := parentRunId(&args)
	if err != nil || value != "run-123.safe" || !reflect.DeepEqual([]string(args), []string{"notify", "--dry-run"}) {
		t.Fatal(value, args, err)
	}
	for _, values := range [][]string{{"--litradar-parent-run-id"}, {"--litradar-parent-run-id", "unsafe/value"}, {"--litradar-parent-run-id", "-bad"}, {"--litradar-parent-run-id", ""}, {"--litradar-parent-run-id", strings.Repeat("x", 129)}, {"--litradar-parent-run-id", "first", "--litradar-parent-run-id", "second"}, {"--help", "--litradar-parent-run-id", "非ASCII"}} {
		args := arguments(values)
		if _, err := parentRunId(&args); err == nil {
			t.Fatal(values)
		}
	}
}

func TestServeArgumentBoundariesAndNoImplicitAuthDatabaseOverride(t *testing.T) {
	configuration, err := parseServeWithBundle([]string{"--secret-key-file", "key", "--port", "+9001", "--host", "0.0.0.0", "--project-root", "fixture", "--scheduler-interval-seconds", "18446744073709551615", "--require-secure-cookies"}, "canonical", "immutable/meta")
	if err != nil || configuration.Port != 9001 || configuration.SchedulerIntervalSeconds != math.MaxUint64 || configuration.Executable != "canonical" || configuration.BundledMetaDir != "immutable/meta" || !configuration.AreSecureCookiesRequired {
		t.Fatal(configuration, err)
	}
	for _, extra := range [][]string{{"--secret-key-file", "second"}, {"--port", "65536"}, {"--port", " 1"}, {"--port=1"}, {"--scheduler-interval-seconds", "0"}, {"--scheduler-interval-seconds", "-1"}, {"--auth-db", "other"}, {"--host", "0.0.0.0", "--development"}, {"--development", "--require-secure-cookies"}, {"--development", "--development"}} {
		args := append([]string{"--secret-key-file", "key"}, extra...)
		if _, err := parseServeWithBundle(args, "canonical", ""); err == nil {
			t.Fatal(args)
		}
	}
	if _, err := parseServeWithBundle([]string{"--development"}, "canonical", ""); err == nil {
		t.Fatal("missing deployment key accepted")
	}
}

func TestOptionParserConsumesOnlyFirstOccurrenceWithoutEqualsOrFlagLookahead(t *testing.T) {
	args := arguments{"--name", "--other", "--name", "second", "--flag", "--flag"}
	value, err := args.take("--name")
	if err != nil || value == nil || *value != "--other" {
		t.Fatal(value, err)
	}
	if !args.flag("--flag") || !reflect.DeepEqual([]string(args), []string{"--name", "second", "--flag"}) {
		t.Fatal(args)
	}
	args = arguments{"--name=value"}
	if value, err := args.take("--name"); value != nil || err != nil {
		t.Fatal(value, err)
	}
}
