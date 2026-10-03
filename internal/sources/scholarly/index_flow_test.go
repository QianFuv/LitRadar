package scholarly

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"regexp"
	"slices"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

type indexObservedTransport struct {
	*FixtureTransport
	last []Attempt
}

func (transport *indexObservedTransport) DrainAttempts() []Attempt {
	transport.last = transport.FixtureTransport.DrainAttempts()
	return transport.last
}

func TestOriginalCompleteIndexWorkflows(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/index-flow-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Id, Input string
			Output    json.RawMessage
		} `json:"observations"`
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	tokenPattern := regexp.MustCompile(`"token":"[0-9a-f]{32}"`)
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var input struct {
				Catalog            domain.JournalCatalogEntry
				Mode               domain.IndexSyncMode
				Fixture            FixtureData
				HasKey             bool `json:"has_key"`
				Anchor, Checkpoint *string
				ReplayAt           []int `json:"replay_at"`
			}
			if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
				t.Fatal(err)
			}
			transport := &indexObservedTransport{FixtureTransport: NewFixtureTransport(input.Fixture)}
			index := NewIndexProvider(transport, input.HasKey, t.TempDir())
			index.now = func() time.Time { return time.Unix(1791072000, 0) }
			checkpoint := input.Checkpoint
			events := []any{}
			for step := 0; step < 80; step++ {
				batch, err := index.Fetch(context.Background(), input.Catalog, domain.IndexFetchContext{Mode: input.Mode, CommittedAnchor: input.Anchor, TraversalCheckpoint: checkpoint})
				if err != nil {
					var failure *provider.Error
					if !errors.As(err, &failure) {
						t.Fatal(err)
					}
					events = append(events, map[string]any{"error": failure.Message, "kind": failure.Kind, "attempts": transport.last})
					break
				}
				if !slices.Contains(input.ReplayAt, step) {
					checkpoint = clonePointer(batch.Progress.Checkpoint)
				}
				if batch.Progress.Checkpoint != nil {
					batch.Progress.Checkpoint = new(tokenPattern.ReplaceAllString(*batch.Progress.Checkpoint, `"token":"00000000000000000000000000000001"`))
				}
				events = append(events, map[string]any{"batch": batch, "attempts": transport.last})
				if batch.Progress.State == domain.Complete {
					break
				}
				if step == 79 {
					t.Fatal("workflow exceeded bound")
				}
			}
			actualBody, err := json.Marshal(map[string]any{"events": events})
			if err != nil {
				t.Fatal(err)
			}
			var actual, expected any
			if decodeIndexComparison(actualBody, &actual) != nil || decodeIndexComparison(observation.Output, &expected) != nil {
				t.Fatal("invalid flow output")
			}
			if !reflect.DeepEqual(actual, expected) {
				actualEvents := actual.(map[string]any)["events"].([]any)
				expectedEvents := expected.(map[string]any)["events"].([]any)
				for event := 0; event < min(len(actualEvents), len(expectedEvents)); event++ {
					if !reflect.DeepEqual(actualEvents[event], expectedEvents[event]) {
						wanted, _ := json.Marshal(expectedEvents[event])
						got, _ := json.Marshal(actualEvents[event])
						t.Fatalf("event %d mismatch\nwant %s\ngot %s", event, wanted, got)
					}
				}
				t.Fatalf("event count mismatch: want %d, got %d", len(expectedEvents), len(actualEvents))
			}
		})
	}
}
