package scholarly

import (
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestOriginalCrossrefWorksetObservations(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/workset-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct{ Id, Kind, Input, Output string } `json:"observations"`
	}
	if json.Unmarshal(body, &corpus) != nil {
		t.Fatal("invalid corpus")
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var actual any
			switch observation.Kind {
			case "checkpoint":
				var state CrossrefCheckpoint
				if json.Unmarshal([]byte(observation.Input), &state) != nil {
					actual = map[string]any{"decoded": false}
					break
				}
				encoded, err := state.MarshalJSON()
				if err != nil {
					t.Fatal(err)
				}
				var validation any
				if err = state.Validate(); err != nil {
					validation = err.Error()
				}
				var query any
				if request, err := state.Query(); err == nil {
					if request.IsEarliest {
						query = map[string]any{"kind": "discover", "until": request.CreatedUntil}
					} else {
						query = map[string]any{"kind": "collect", "from": request.CreatedFrom, "until": request.CreatedUntil, "updated_from": request.UpdatedFrom, "updated_until": request.UpdatedUntil, "cursor": request.Cursor}
					}
				}
				actual = map[string]any{"decoded": true, "encoded": string(encoded), "validation": validation, "query": query}
			case "anchor":
				var anchor Anchor
				if json.Unmarshal([]byte(observation.Input), &anchor) != nil {
					actual = map[string]any{"decoded": false}
					break
				}
				encoded, err := anchor.MarshalJSON()
				if err != nil {
					t.Fatal(err)
				}
				actual = map[string]any{"decoded": true, "encoded": string(encoded), "valid": anchor.IsValid()}
			case "work":
				work, err := transport.ParseJson([]byte(observation.Input))
				if err != nil {
					t.Fatal(err)
				}
				created, err := createdSecond(work)
				var createdValue, createdError any
				if err == nil {
					createdValue = strconv.FormatInt(created, 10)
				} else {
					createdError = err.Error()
				}
				var anchor any
				if value := CrossrefIssueAnchor(work); value != nil {
					encoded, err := value.MarshalJSON()
					if err != nil {
						t.Fatal(err)
					}
					anchor = string(encoded)
				}
				payload := consumedPayload(work)
				serialized, err := domain.Json(payload)
				if err != nil {
					t.Fatal(err)
				}
				key := workKey(payload, string(serialized))
				order, err := crossrefOrder(payload, key)
				if err != nil {
					t.Fatal(err)
				}
				actual = map[string]any{"created": createdValue, "created_error": createdError, "date": CrossrefDate(work), "anchor": anchor, "payload": string(serialized), "key": key, "order": map[string]any{"date": order.Date, "fingerprint": order.Fingerprint, "anchor": order.Anchor, "year": order.Year, "volume": order.Volume, "issue": order.Issue}}
			default:
				t.Fatal("unknown observation")
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			got, err := transport.ParseJson(encoded)
			if err != nil {
				t.Fatal(err)
			}
			want, err := transport.ParseJson([]byte(observation.Output))
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("want %s\ngot %s", observation.Output, encoded)
			}
		})
	}
}
func TestCrossrefMetadataOwnsMutableInputs(t *testing.T) {
	initial := map[string]any{"title": []any{"original"}, "author": []any{map[string]any{"given": "A"}}}
	retained := consumedPayload(initial)
	retained["title"].([]any)[0] = "changed"
	retained["author"].([]any)[0].(map[string]any)["given"] = "changed"
	if initial["title"].([]any)[0] != "original" || initial["author"].([]any)[0].(map[string]any)["given"] != "A" {
		t.Fatal("retained payload aliases caller")
	}
	state, err := NewCrossrefCheckpoint("1234-5679", 10, nil)
	if err != nil {
		t.Fatal(err)
	}
	state.CreatedFrom = clonePointer(new(int64(1)))
	state.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: 1, Until: 10, Cursor: clonePointer(new("cursor"))}
	copied := state.Clone()
	*copied.CreatedFrom = 2
	*copied.Phase.Cursor = "other"
	if *state.CreatedFrom != 1 || *state.Phase.Cursor != "cursor" {
		t.Fatal("checkpoint aliases caller")
	}
}
