package cfp

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"testing"
	"time"
)

func TestOriginalDomainObservations(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/cfp/storage-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Domain []struct {
			Name  string
			Input struct {
				Op, Raw, Text string
				Stage         json.RawMessage
				Times         []string
			}
			Expected json.RawMessage
		}
	}
	if err = json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Domain) < 168 {
		t.Fatal("missing independent domain observations")
	}
	for _, entry := range fixture.Domain {
		t.Run(entry.Name, func(t *testing.T) {
			var result any
			var err error
			switch entry.Input.Op {
			case "dates":
				stage := Paper
				if len(entry.Input.Stage) > 0 {
					err = json.Unmarshal(entry.Input.Stage, &stage)
				}
				if err == nil {
					result = ParseDates(entry.Input.Text, stage)
				}
			case "source":
				var source Source
				err = json.Unmarshal([]byte(entry.Input.Raw), &source)
				if err == nil {
					notice := ParseSource(source)
					var states []State
					if notice != nil {
						states = []State{}
						for _, instant := range entry.Input.Times {
							now, parseErr := time.Parse(time.RFC3339, instant)
							if parseErr != nil {
								t.Fatal(parseErr)
							}
							states = append(states, notice.State(now))
						}
					}
					result = map[string]any{"source": source, "notice": notice, "states": states}
				}
			case "seed":
				var seed Seed
				err = json.Unmarshal([]byte(entry.Input.Raw), &seed)
				result = seed
			case "notice":
				var notice Notice
				err = json.Unmarshal([]byte(entry.Input.Raw), &notice)
				result = notice
			default:
				t.Fatal("unknown observer")
			}
			if err != nil {
				result = map[string]string{"error": "json"}
			}
			actual, err := json.Marshal(result)
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
			if !reflect.DeepEqual(decode(actual), decode(entry.Expected)) {
				t.Fatalf("input %s %s\ngot %s\nwant %s", entry.Input.Raw, entry.Input.Text, actual, entry.Expected)
			}
		})
	}
}
