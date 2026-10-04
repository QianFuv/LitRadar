package logfilter

import (
	"encoding/json"
	"os"
	"testing"
)

func TestFieldRegexMatchesLockedRustObservations(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/runtime/log-regex-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Pattern      string
			Valid        bool
			Observations []struct {
				Input   string
				Matched bool
			}
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	if len(corpus.Cases) < 50 {
		t.Fatal("missing frozen field matcher coverage")
	}
	for _, item := range corpus.Cases {
		t.Run(item.Pattern, func(t *testing.T) {
			expression, err := compileFieldRegex(item.Pattern)
			if (err == nil) != item.Valid {
				t.Fatalf("compile validity: %v want valid=%v", err, item.Valid)
			}
			if err != nil {
				return
			}
			for _, observation := range item.Observations {
				if actual := expression.matches(observation.Input); actual != observation.Matched {
					t.Errorf("%q: got %v, want %v", observation.Input, actual, observation.Matched)
				}
			}
		})
	}
}
