package scholarly

import (
	"encoding/json"
	"os"
	"strconv"
	"testing"
)

func TestFrozenScholarlyDecode(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/scholarly-decode-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures struct {
		Observations []struct {
			Kind, Input string
			Output      struct{ Valid bool }
		}
	}
	if err := json.Unmarshal(body, &fixtures); err != nil {
		t.Fatal(err)
	}
	for index, observation := range fixtures.Observations {
		t.Run(observation.Kind+"/"+strconv.Itoa(index), func(t *testing.T) {
			var err error
			if observation.Kind == "fixture_decode" {
				var value FixtureData
				err = json.Unmarshal([]byte(observation.Input), &value)
			} else {
				var value LiveConfig
				err = json.Unmarshal([]byte(observation.Input), &value)
			}
			if (err == nil) != observation.Output.Valid {
				t.Fatalf("valid=%v want=%v: %s (%v)", err == nil, observation.Output.Valid, observation.Input, err)
			}
		})
	}
}
