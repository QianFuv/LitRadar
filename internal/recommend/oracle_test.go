package recommend

import (
	"encoding/json"
	"errors"
	"math/big"
	"os"
	"reflect"
	"runtime"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestOriginalRecommendationObservations(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/delivery/recommend-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct{ Name, Input, Output string }
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, vector := range corpus.Cases {
		t.Run(vector.Name, func(t *testing.T) {
			var input struct {
				Op         string                     `json:"op"`
				Raw        string                     `json:"raw"`
				Kind       PayloadKind                `json:"kind"`
				Response   json.RawMessage            `json:"response"`
				Subscriber domain.Subscriber          `json:"subscriber"`
				Candidates []storage.ArticleCandidate `json:"candidates"`
				Selection  domain.SelectionResult     `json:"selection"`
				Dedupe     map[string]string          `json:"dedupe"`
				Global     GlobalConfig               `json:"global"`
				Defaults   Defaults                   `json:"defaults"`
				Override   *string                    `json:"override"`
				Database   string                     `json:"database"`
				Run        string                     `json:"run"`
				Selected   []string                   `json:"selected"`
				Previous   map[string]int64           `json:"previous"`
				Current    map[string]int64           `json:"current"`
			}
			if err := json.Unmarshal([]byte(vector.Input), &input); err != nil {
				t.Fatal(err)
			}
			var actual any
			switch input.Op {
			case "manifest":
				value, err := ParseChangeManifest([]byte(input.Raw), input.Database)
				if errors.Is(err, ErrManifestJson) {
					actual = map[string]any{"error_kind": "json"}
				} else if err != nil {
					actual = map[string]any{"error": err.Error()}
				} else {
					actual = map[string]any{"value": value}
				}
			case "checkpoint":
				value, err := ParseSnapshot([]byte(input.Raw))
				if err != nil {
					actual = map[string]any{"error": err.Error()}
				} else {
					actual = map[string]any{"value": value}
				}
			case "payload":
				response, err := transport.ParseJson(input.Response)
				if err != nil {
					t.Fatal(err)
				}
				value, err := ExtractResponsePayload(response, input.Kind)
				if err != nil {
					actual = map[string]any{"error": err.Error()}
				} else {
					actual = map[string]any{"value": value}
				}
			case "selection":
				matches := make([]int64, 0, len(input.Candidates))
				for _, candidate := range input.Candidates {
					matches = append(matches, CandidateMatchScore(candidate, input.Subscriber))
				}
				actual = map[string]any{"accepted": ApplySelectionRules(input.Selection, input.Subscriber, CandidatesById(input.Candidates), input.Dedupe), "matches": matches, "has_preferences": HasSelectionPreferences(input.Subscriber), "deduped": DeduplicateCandidates(input.Candidates)}
			case "message":
				actual = map[string]any{"title": BuildMessageTitle(input.Database, input.Run), "content": BuildMarkdownContent(input.Database, input.Run, input.Subscriber, input.Selection.Summary, input.Selection.Selections, CandidatesById(input.Candidates))}
			case "config":
				actual = ResolveAiRuntimeConfigs(input.Subscriber, input.Global, input.Defaults, input.Override)
			case "snapshot":
				actual = map[string]any{"issues": ComputeChangedIssueKeys(input.Previous, input.Current), "inpress": ComputeChangedInpressKeys(input.Previous, input.Current)}
			case "database":
				actual = IsDatabaseSelected(input.Selected, input.Database)
				if runtime.GOOS != "windows" && strings.HasPrefix(vector.Name, "verbatim-") {
					return
				}
				if runtime.GOOS != "windows" && input.Database == `C:\data\db.sqlite` {
					if actual != false {
						t.Fatal("Unix must retain backslashes")
					}
					return
				}
			default:
				t.Fatal(input.Op)
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			actualValue, err := transport.ParseJson(encoded)
			if err != nil {
				t.Fatal(err)
			}
			expectedValue, err := transport.ParseJson([]byte(vector.Output))
			if err != nil {
				t.Fatal(err)
			}
			if !equalObservation(actualValue, expectedValue) {
				t.Errorf("got %s\nwant %s", encoded, vector.Output)
			}
		})
	}
}

func equalObservation(actual, expected any) bool {
	switch value := actual.(type) {
	case json.Number:
		other, ok := expected.(json.Number)
		if !ok {
			return false
		}
		left, leftOk := new(big.Rat).SetString(value.String())
		right, rightOk := new(big.Rat).SetString(other.String())
		return leftOk && rightOk && left.Cmp(right) == 0
	case map[string]any:
		other, ok := expected.(map[string]any)
		if !ok || len(value) != len(other) {
			return false
		}
		for key, child := range value {
			target, exists := other[key]
			if !exists || !equalObservation(child, target) {
				return false
			}
		}
		return true
	case []any:
		other, ok := expected.([]any)
		if !ok || len(value) != len(other) {
			return false
		}
		for index, child := range value {
			if !equalObservation(child, other[index]) {
				return false
			}
		}
		return true
	default:
		return reflect.DeepEqual(actual, expected)
	}
}
