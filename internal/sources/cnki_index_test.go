package sources

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

func TestOriginalCnkiIndexConversion(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/cnki-index-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Id, Kind, Input string
			Output          json.RawMessage
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var actual any
			var operationErr error
			switch observation.Kind {
			case "cnki_anchor":
				state, err := decodeCnkiAnchor(observation.Input)
				operationErr = err
				if err == nil {
					encoded, err := state.Encode()
					if err != nil {
						t.Fatal(err)
					}
					actual = map[string]any{"encoded": encoded}
				}
			case "cnki_checkpoint":
				state, err := decodeCnkiCheckpoint(observation.Input)
				operationErr = err
				if err == nil {
					encoded, err := state.Encode()
					if err != nil {
						t.Fatal(err)
					}
					actual = map[string]any{"encoded": encoded}
				}
			case "cnki_article":
				var fields map[string]json.RawMessage
				if err := json.Unmarshal([]byte(observation.Input), &fields); err != nil {
					t.Fatal(err)
				}
				var catalog domain.JournalCatalogEntry
				if err := json.Unmarshal(fields["catalog"], &catalog); err != nil {
					t.Fatal(err)
				}
				values := map[string]any{}
				for _, key := range []string{"issue", "summary", "detail"} {
					value, err := transport.ParseJson(fields[key])
					if err != nil {
						t.Fatal(err)
					}
					values[key] = value
				}
				issue := cnkiIssueDraft(catalog, values["issue"])
				actual = map[string]any{"issue": issue, "article": cnkiArticleDraft(catalog, issue, values["summary"], values["detail"]), "skip": cnkiLacksAuthorsAndDoi(values["summary"], values["detail"])}
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
			assertCnkiOracle(t, actual, observation.Output)
		})
	}
}

func assertCnkiOracle(t *testing.T, actual any, expectedBody []byte) {
	t.Helper()
	actualBody, err := json.Marshal(actual)
	if err != nil {
		t.Fatal(err)
	}
	values := make([]any, 2)
	for index, body := range [][]byte{actualBody, expectedBody} {
		decoder := json.NewDecoder(bytes.NewReader(body))
		decoder.UseNumber()
		if err := decoder.Decode(&values[index]); err != nil {
			t.Fatal(err)
		}
	}
	if !reflect.DeepEqual(values[0], values[1]) {
		t.Fatalf("want %s\ngot %s", expectedBody, actualBody)
	}
}
