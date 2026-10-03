package scholarly

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"
)

func TestFrozenScholarlySequences(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/scholarly-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Observations []struct {
			Id                    string
			Fixture               FixtureData
			HasSemanticScholarKey bool `json:"has_semantic_scholar_key"`
			Operations            []struct {
				Op           string
				Issn, Title  string
				Issns, Dois  []string
				SourceId     string `json:"source_id"`
				Date, Cursor *string
				BatchSize    int `json:"batch_size"`
				Value        any
				Query        struct {
					Kind         string
					Until        int64
					CreatedFrom  int64   `json:"created_from"`
					CreatedUntil int64   `json:"created_until"`
					UpdatedFrom  *string `json:"updated_from"`
					UpdatedUntil *int64  `json:"updated_until"`
					Cursor       *string
				}
			}
			Output []any
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	if err := decoder.Decode(&fixture); err != nil {
		t.Fatal(err)
	}
	for _, observation := range fixture.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			transport := NewFixtureTransport(observation.Fixture)
			client := NewClient(transport, observation.HasSemanticScholarKey)
			for index, operation := range observation.Operations {
				var value any
				var err error
				switch operation.Op {
				case "crossref":
					query := operation.Query
					until := query.CreatedUntil
					if query.Kind == "earliest_created" {
						until = query.Until
					}
					value, err = client.FetchCrossrefPage(context.Background(), operation.Issn, CrossrefQuery{IsEarliest: query.Kind == "earliest_created", CreatedFrom: query.CreatedFrom, CreatedUntil: until, UpdatedFrom: query.UpdatedFrom, UpdatedUntil: query.UpdatedUntil, Cursor: query.Cursor})
				case "issns":
					value, err = client.FetchOpenAlexSourceByIssns(context.Background(), operation.Issns)
				case "title":
					value, err = client.FetchOpenAlexSourceByTitle(context.Background(), operation.Title)
				case "source_page":
					value, err = client.FetchOpenAlexWorksBySourcePage(context.Background(), operation.SourceId, operation.Date, operation.Cursor)
				case "openalex_dois":
					value, err = client.FetchOpenAlexByDois(context.Background(), operation.Dois, operation.BatchSize)
				case "s2_dois":
					value, err = client.FetchSemanticScholarByDois(context.Background(), operation.Dois, operation.BatchSize)
				case "drain":
					value = client.DrainAttempts()
				case "normalize":
					value = NormalizeDoi(operation.Value)
				default:
					t.Fatalf("unknown operation %s", operation.Op)
				}
				result := map[string]any{"ok": value}
				if err != nil {
					var failure *Error
					if !errors.As(err, &failure) {
						t.Fatal(err)
					}
					detail := map[string]any{"kind": failure.Kind, "display": failure.Error()}
					switch failure.Kind {
					case "HttpStatus":
						detail["service"] = failure.Service
						detail["endpoint"] = failure.Endpoint
						detail["status_code"] = failure.StatusCode
						detail["body"] = failure.Body
					case "Request":
						detail["service"] = failure.Service
						detail["endpoint"] = failure.Endpoint
						detail["message"] = failure.Message
					default:
						detail["message"] = failure.Message
					}
					result = map[string]any{"error": detail}
				}
				actual := normalizedTestJson(t, map[string]any{"result": result, "attempts": client.Attempts(), "captures": transport.Captures()})
				if !reflect.DeepEqual(actual, observation.Output[index]) {
					encoded, _ := json.Marshal(actual)
					expected, _ := json.Marshal(observation.Output[index])
					t.Fatalf("operation %d %s\ngot %s\nwant %s", index, operation.Op, encoded, expected)
				}
			}
		})
	}
}

func normalizedTestJson(t *testing.T, value any) any {
	t.Helper()
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	var result any
	if err := decoder.Decode(&result); err != nil {
		t.Fatal(err)
	}
	return result
}
