package scheduler

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestSchedulerStateWireAndLegacyMapping(t *testing.T) {
	for _, state := range []State{Idle, Pending, Claimed, Running, Success, Failed, TimedOut, Error, Unknown, Cancelled} {
		encoded, err := json.Marshal(state)
		if err != nil || string(encoded) != `"`+string(state)+`"` {
			t.Fatalf("%s: %s %v", state, encoded, err)
		}
		for _, raw := range []string{string(encoded), `{` + string(encoded) + `:null}`} {
			var decoded State
			if err := json.Unmarshal([]byte(raw), &decoded); err != nil || decoded != state {
				t.Fatalf("%s: %s %v", raw, decoded, err)
			}
		}
	}
	for _, raw := range []string{`"timeout"`, `""`, `"legacy"`, `null`, `{}`, `[]`, `{"success":null,"failed":null}`, `{"success":null,"success":null}`, `{"success":true}`} {
		var state State
		if json.Unmarshal([]byte(raw), &state) == nil {
			t.Fatalf("invalid wire state accepted: %s", raw)
		}
	}
	if PersistedState("") != Idle || PersistedState("legacy") != Unknown || !Cancelled.IsTerminal() || Running.IsTerminal() {
		t.Fatal("legacy or terminal mapping changed")
	}
}

func TestSchedulerValidationIsDistinctFromStoredJsonFailure(t *testing.T) {
	var validation *ValidationError
	if !errors.As(ValidateTiming("Factory", 60), &validation) {
		t.Fatal("invalid timing is not a public validation error")
	}
	var job Job
	err := json.Unmarshal([]byte(`{"kind":"unknown"}`), &job)
	if err == nil || errors.As(err, &validation) {
		t.Fatal("corrupt stored job was misclassified as input validation")
	}
}
