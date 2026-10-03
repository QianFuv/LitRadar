package scholarly

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func decodeIndexComparison(body []byte, value any) error {
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	return decoder.Decode(value)
}

func TestOriginalIndexStateAndWindow(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/index-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Id, Kind, Input string
			Output          json.RawMessage
		} `json:"observations"`
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var actual any
			var operationErr error
			switch observation.Kind {
			case "checkpoint":
				var state indexCheckpoint
				state, operationErr = decodeIndexCheckpoint(observation.Input, new("2026-10-04"))
				if operationErr == nil {
					encoded, err := encodeIndexCheckpoint(state)
					if err != nil {
						t.Fatal(err)
					}
					actual = map[string]any{"encoded": encoded}
				}
			case "window":
				var input struct {
					Window        indexWindow
					Anchors       []*Anchor
					Unknown, Next bool
				}
				if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
					t.Fatal(err)
				}
				var plan indexPagePlan
				plan, operationErr = planIndexPageWindow(input.Window, input.Anchors, input.Unknown, input.Next)
				if operationErr == nil {
					window, err := encodeWorksetStruct(plan.Window)
					if err != nil {
						t.Fatal(err)
					}
					actual = map[string]any{"selected": plan.SelectedIndices, "window": string(window), "progress": plan.Progress}
				}
			case "context":
				var input struct {
					Mode               domain.IndexSyncMode
					Anchor, Checkpoint *string
				}
				if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
					t.Fatal(err)
				}
				var window indexWindow
				var source *indexSource
				window, source, operationErr = indexWindowFromContext(domain.IndexFetchContext{Mode: input.Mode, CommittedAnchor: input.Anchor, TraversalCheckpoint: input.Checkpoint}, new("2026-10-04"))
				if operationErr == nil {
					encoded, err := encodeWorksetStruct(window)
					if err != nil {
						t.Fatal(err)
					}
					var encodedSource any
					if source != nil {
						body, err := source.MarshalJSON()
						if err != nil {
							t.Fatal(err)
						}
						encodedSource = string(body)
					}
					actual = map[string]any{"window": string(encoded), "source": encodedSource, "filter": indexWindowFilter(window, new("2026-10-04"))}
				}
			case "article":
				var input struct {
					Catalog                         domain.JournalCatalogEntry
					Provider                        string
					Work, OpenAlex, SemanticScholar json.RawMessage `json:"-"`
				}
				var fields map[string]json.RawMessage
				if err := json.Unmarshal([]byte(observation.Input), &fields); err != nil {
					t.Fatal(err)
				}
				if err := json.Unmarshal(fields["catalog"], &input.Catalog); err != nil {
					t.Fatal(err)
				}
				if err := json.Unmarshal(fields["provider"], &input.Provider); err != nil {
					t.Fatal(err)
				}
				decode := func(key string) any {
					body := fields[key]
					if len(body) == 0 {
						return nil
					}
					value, err := transport.ParseJson(body)
					if err != nil {
						t.Fatal(err)
					}
					return value
				}
				var article *domain.ArticleDraft
				if input.Provider == "crossref" {
					article = crossrefArticle(input.Catalog, decode("work"), decode("openalex"), decode("semantic_scholar"))
				} else {
					article = openAlexArticle(input.Catalog, decode("work"))
				}
				actual = map[string]any{"article": article}
			case "scope":
				var input struct {
					Catalog domain.JournalCatalogEntry
					Window  indexWindow
				}
				if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
					t.Fatal(err)
				}
				scope, err := indexWorksetScope(input.Catalog, input.Window)
				if err != nil {
					t.Fatal(err)
				}
				actual = map[string]any{"scope": scope}
			default:
				t.Fatal(observation.Kind)
			}
			if operationErr != nil {
				var failure *provider.Error
				if !errors.As(operationErr, &failure) {
					t.Fatal(operationErr)
				}
				actual = map[string]any{"error": failure.Message, "kind": failure.Kind}
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var comparable, expected any
			if decodeIndexComparison(encoded, &comparable) != nil || decodeIndexComparison(observation.Output, &expected) != nil {
				t.Fatal("invalid oracle value")
			}
			if !reflect.DeepEqual(comparable, expected) {
				t.Fatalf("want %s\ngot %s", observation.Output, encoded)
			}
		})
	}
}
