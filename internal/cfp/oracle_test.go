package cfp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strconv"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
)

type observedCall struct {
	Url    string
	Result json.RawMessage
}
type oracleTransport struct {
	testing *testing.T
	calls   []observedCall
	next    int
}

func (transport *oracleTransport) Fetch(_ context.Context, _ SourceConfig, url string, _ time.Time) (Document, error) {
	transport.testing.Helper()
	if transport.next >= len(transport.calls) {
		transport.testing.Fatal("unexpected fetch", url)
	}
	call := transport.calls[transport.next]
	transport.next++
	if call.Url != url {
		transport.testing.Fatalf("fetch %s want %s", url, call.Url)
	}
	var failure struct{ Error string }
	if err := json.Unmarshal(call.Result, &failure); err != nil {
		transport.testing.Fatal(err)
	}
	if failure.Error != "" {
		return Document{}, errors.New(failure.Error)
	}
	var document Document
	if err := json.Unmarshal(call.Result, &document); err != nil {
		transport.testing.Fatal(err)
	}
	return document, nil
}

func TestOriginalSourceObservations(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/cfp/source-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Name  string
			Input struct {
				Op        string
				Config    SourceConfig
				Document  Document
				Source    domain.Source
				Checked   string
				Discovery bool
				Calls     []observedCall
			}
			Expected json.RawMessage
		}
	}
	if err = json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 70 {
		t.Fatal("missing source observations")
	}
	for index, entry := range fixture.Cases {
		t.Run(strconv.Itoa(index)+"-"+entry.Name, func(t *testing.T) {
			var result any
			var err error
			input := entry.Input
			switch input.Op {
			case "parse":
				result, err = ParsePage(input.Config, input.Document, input.Checked, input.Discovery)
			case "full":
				result, err = ExtractFullText(input.Source, input.Document)
			case "links":
				result = OriginalLinks(input.Source, input.Document)
			case "challenge":
				result = IsChallenge(input.Document)
			case "acquire":
				transport := &oracleTransport{testing: t, calls: input.Calls}
				result, err = Acquire(context.Background(), transport, input.Config, input.Checked, time.Now().Add(5*time.Second))
				if transport.next != len(input.Calls) {
					t.Fatal("missing required detail calls")
				}
			default:
				t.Fatal("unknown observation")
			}
			if err != nil {
				result = map[string]string{"error": err.Error()}
			}
			encoded, err := json.Marshal(result)
			if err != nil {
				t.Fatal(err)
			}
			decode := func(data []byte) any {
				var value any
				decoder := json.NewDecoder(bytes.NewReader(data))
				decoder.UseNumber()
				if err := decoder.Decode(&value); err != nil {
					t.Fatal(err)
				}
				return value
			}
			if !reflect.DeepEqual(decode(encoded), decode(entry.Expected)) {
				t.Fatalf("got %s\nwant %s", encoded, entry.Expected)
			}
		})
	}
}
