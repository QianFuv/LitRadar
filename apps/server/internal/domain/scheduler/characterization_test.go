package scheduler

import (
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
)

// populatedSchedulerJob exposes failed decode mutation and replacement of old optional values.
func populatedSchedulerJob() Job {
	metadata, database, count := "old.csv", "old.sqlite", uint64(9)
	return Job{Kind: "index", MetadataFile: &metadata, Notify: true, Push: true, Database: &database, MaxCandidates: &count}
}

// TestScheduledJobDecodingPublishesOnlyCompleteReplacement checks atomic strict-JSON admission.
func TestScheduledJobDecodingPublishesOnlyCompleteReplacement(t *testing.T) {
	original := populatedSchedulerJob()
	for _, raw := range []string{`null`, `[]`, `{}`, `{"kind":"index","kind":"index"}`, `{"kind":"index","\u006bind":"index"}`, `{"kind":"index","notify":null}`, `{"kind":"index","database":null}`, `{"kind":"notify","push":false}`, `{"kind":"push","max_candidates":-0}`, `{"kind":"push","max_candidates":1.0}`, `{"kind":"push","max_candidates":1e0}`, `{"kind":"push","max_candidates":18446744073709551616}`, `{"kind":"index","metadata_file":"\ud800"}`, `{"kind":"unknown"}`, `{"kind":"index","unknown":1}`} {
		actual := original
		err := actual.UnmarshalJSON([]byte(raw))
		if err == nil || err.Error() != "invalid scheduled job JSON" || !reflect.DeepEqual(actual, original) {
			t.Fatalf("%s: %+v %v", raw, actual, err)
		}
	}
	for _, raw := range []string{`{"kind":"index"}`, `{"kind":"notify","database":null,"max_candidates":null}`, `{"kind":"push"}`} {
		actual := original
		if err := actual.UnmarshalJSON([]byte(raw)); err != nil {
			t.Fatal(raw, err)
		}
		assertScheduledJobReplacement(t, actual)
	}
}

// assertScheduledJobReplacement checks variant omission/default order and clearing of previous state.
func assertScheduledJobReplacement(t *testing.T, actual Job) {
	t.Helper()
	if !reflect.DeepEqual(actual, Job{Kind: actual.Kind}) {
		t.Fatal(actual)
	}
	encoded, err := json.Marshal(actual)
	expected := `{"kind":"` + actual.Kind + `"}`
	if actual.Kind == "index" {
		expected = `{"kind":"index","notify":false,"push":false}`
	}
	if err != nil || string(encoded) != expected {
		t.Fatal(string(encoded), err)
	}
}

// TestScheduledJobDecodingKeepsBusinessValidationSeparate checks semantic admission and rejection priority.
func TestScheduledJobDecodingKeepsBusinessValidationSeparate(t *testing.T) {
	for _, raw := range []string{`{"kind":"index","metadata_file":"../x.csv"}`, `{"kind":"push","database":"safe.sqlite","max_candidates":0}`} {
		var job Job
		if err := json.Unmarshal([]byte(raw), &job); err != nil {
			t.Fatal(err)
		}
		var validation *ValidationError
		if !errors.As(job.Validate(), &validation) {
			t.Fatal(job)
		}
	}
	database := "bad"
	count := uint64(0)
	err := (Job{Kind: "push", Database: &database, MaxCandidates: &count}).Validate()
	if err == nil || err.Error() != "database must be a safe .sqlite basename" {
		t.Fatal(err)
	}
	if err := ValidateTiming("Factory", 0); err == nil || err.Error() != "timezone must be a valid IANA name" {
		t.Fatal(err)
	}
}

// TestScheduledFilenameRetainsRawByteBoundaries checks exact case and unnormalized basename grammar.
func TestScheduledFilenameRetainsRawByteBoundaries(t *testing.T) {
	for _, item := range []struct {
		name    string
		allowed bool
	}{
		{"x.csv", true}, {"_x.csv", true}, {"-x.csv", true}, {strings.Repeat("a", 124) + ".csv", true},
		{strings.Repeat("a", 125) + ".csv", false}, {".csv", false}, {"x.CSV", false}, {" x.csv", false}, {"a..csv", false}, {"中.csv", false}, {"x/\x00.csv", false},
	} {
		err := validateFilename(&item.name, ".csv", "metadata file")
		if (err == nil) != item.allowed {
			t.Fatal(item, err)
		}
	}
	if err := validateFilename(nil, ".csv", "metadata file"); err != nil {
		t.Fatal(err)
	}
}

// TestSchedulerStateInvalidWirePreservesReceiver checks strict state admission before publication.
func TestSchedulerStateInvalidWirePreservesReceiver(t *testing.T) {
	for _, raw := range []string{`null`, `{}`, `{"success":null,"failed":null}`, `{"success":true}`, `"legacy"`, `{"success":null} null`} {
		state := Running
		if err := state.UnmarshalJSON([]byte(raw)); err == nil || state != Running {
			t.Fatal(raw, state, err)
		}
	}
}
