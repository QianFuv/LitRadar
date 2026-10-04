package index

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"
)

func TestOriginalRustWorkerWire(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/index/worker-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Kind, Name, Payload string
				Stream              bool
				Reads               int
			}
			Expected json.RawMessage
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, item := range corpus.Observations {
		t.Run(item.Input.Kind+"/"+item.Input.Name, func(t *testing.T) {
			target := func() any {
				switch item.Input.Kind {
				case "request":
					return &WorkerRequest{}
				case "assignment":
					return &WorkerAssignment{}
				case "bootstrap":
					return &WorkerBootstrap{}
				case "worker":
					return &WorkerMessage{}
				case "parent":
					return &ParentMessage{}
				case "failure":
					return &WorkerFailure{}
				}
				panic("kind")
			}
			var actual any
			if item.Input.Stream {
				reader := NewProtocolReader(strings.NewReader(item.Input.Payload))
				results := []any{}
				for count := 0; count < item.Input.Reads; count++ {
					value := target()
					if err := reader.Read(value); err != nil {
						results = append(results, map[string]any{"error": err.Error()})
					} else {
						results = append(results, map[string]any{"value": value})
					}
				}
				actual = results
			} else {
				value := target()
				if err := json.Unmarshal([]byte(item.Input.Payload), value); err != nil {
					actual = map[string]any{"error": "invalid"}
				} else {
					actual = map[string]any{"value": value}
				}
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			decoder := json.NewDecoder(bytes.NewReader(encoded))
			decoder.UseNumber()
			if err := decoder.Decode(&got); err != nil {
				t.Fatal(err)
			}
			decoder = json.NewDecoder(bytes.NewReader(item.Expected))
			decoder.UseNumber()
			if err := decoder.Decode(&want); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("actual=%s\nexpected=%s", encoded, item.Expected)
			}
		})
	}
}

func TestWorkerDiagnosticsExcludeOpaqueState(t *testing.T) {
	secret := "private-token-and-cursor"
	values := []any{WorkerBootstrap{CnkiCaptchaToken: &secret, ProviderProxyUrl: &secret, ScholarlyWorksetDir: &secret}, WorkerAssignment{CommittedAnchor: &secret, TraversalCheckpoint: &secret}, WorkerRequest{Assignments: []WorkerAssignment{{CommittedAnchor: &secret}}}}
	for _, value := range values {
		for _, format := range []string{"%v", "%+v", "%#v"} {
			if strings.Contains(fmt.Sprintf(format, value), secret) {
				t.Fatalf("leaked with %s", format)
			}
		}
	}
}
