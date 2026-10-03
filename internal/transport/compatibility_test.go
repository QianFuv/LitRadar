package transport

import (
	"encoding/json"
	"os"
	"strconv"
	"testing"
	"time"
)

func TestFrozenRustTransport(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/transport-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Observations []struct {
			Kind   string `json:"kind"`
			Input  string `json:"input"`
			Now    string `json:"now"`
			Bytes  []int  `json:"bytes"`
			Output struct {
				Delay *struct {
					Seconds     string `json:"seconds"`
					Nanoseconds uint32 `json:"nanoseconds"`
				} `json:"delay"`
				Text  string `json:"text"`
				Valid bool   `json:"valid"`
			} `json:"output"`
		} `json:"observations"`
	}
	if err := json.Unmarshal(body, &fixture); err != nil {
		t.Fatal(err)
	}
	for index, observation := range fixture.Observations {
		t.Run(observation.Kind+"/"+strconv.Itoa(index), func(t *testing.T) {
			switch observation.Kind {
			case "retry":
				now, err := time.Parse(time.RFC3339Nano, observation.Now)
				if err != nil {
					t.Fatal(err)
				}
				delay, isValid := ParseRetryAfter(observation.Input, now)
				if isValid != (observation.Output.Delay != nil) {
					t.Fatalf("%q at %s: valid=%v want=%v", observation.Input, observation.Now, isValid, observation.Output.Delay != nil)
				}
				if isValid && (strconv.FormatUint(delay.Seconds, 10) != observation.Output.Delay.Seconds || delay.Nanoseconds != observation.Output.Delay.Nanoseconds) {
					t.Fatalf("%q at %s: delay=%+v want=%+v", observation.Input, observation.Now, delay, observation.Output.Delay)
				}
			case "text":
				input := make([]byte, len(observation.Bytes))
				for index, value := range observation.Bytes {
					input[index] = byte(value)
				}
				if actual := LossyUtf8(input); actual != observation.Output.Text {
					t.Fatalf("%x: %q want %q", input, actual, observation.Output.Text)
				}
			case "json":
				_, err := ParseJson([]byte(observation.Input))
				if (err == nil) != observation.Output.Valid {
					t.Fatalf("JSON %q: %v", observation.Input, err)
				}
			case "proxy":
				_, err := ExplicitProxy(observation.Input)
				if (err == nil) != observation.Output.Valid {
					t.Fatalf("proxy %q: %v, want valid=%v", observation.Input, err, observation.Output.Valid)
				}
			default:
				t.Fatal("unknown observation")
			}
		})
	}
}
