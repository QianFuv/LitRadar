package sources

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"slices"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
)

func TestOriginalCnkiIndexWorkflows(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/cnki-flow-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Id, Input string
			Output    json.RawMessage
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var input struct {
				Catalog            domain.JournalCatalogEntry
				Fixture            cnki.FixtureData
				Mode               domain.IndexSyncMode
				Anchor, Checkpoint *string
				Workers            int
				ReplayAt           []int `json:"replay_at"`
			}
			if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
				t.Fatal(err)
			}
			index, err := NewCnkiIndexProviderWithWorkers(cnki.NewFixtureTransport(input.Fixture), max(input.Workers, 1))
			if err != nil {
				t.Fatal(err)
			}
			defer index.Close()
			index.sleep = func(context.Context, time.Duration) error { return nil }
			checkpoint := input.Checkpoint
			events := []any{}
			for step := 0; step < 80; step++ {
				batch, err := index.Fetch(context.Background(), input.Catalog, domain.IndexFetchContext{Mode: input.Mode, CommittedAnchor: input.Anchor, TraversalCheckpoint: checkpoint})
				if err != nil {
					var failure *provider.Error
					if !errors.As(err, &failure) {
						t.Fatal(err)
					}
					events = append(events, map[string]any{"error": failure.Message, "kind": failure.Kind})
					break
				}
				events = append(events, map[string]any{"batch": batch})
				if !slices.Contains(input.ReplayAt, step) {
					checkpoint = batch.Progress.Checkpoint
				}
				if batch.Progress.State == domain.Complete {
					break
				}
				if step == 79 {
					t.Fatal("workflow exceeded bound")
				}
			}
			assertCnkiOracle(t, map[string]any{"events": events}, observation.Output)
		})
	}
}
