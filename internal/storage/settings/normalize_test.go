package settings

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestNormalizeMatchesIndependentRustSettings(t *testing.T) {
	checkRustObservations(t, "settings-vectors.json")
}

func TestUrlsMatchIndependentRustCorpus(t *testing.T) {
	checkRustObservations(t, "url-vectors.json")
}

func checkRustObservations(t *testing.T, filename string) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "migration", "auth", filename))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Field  string
			Input  string
			Output *string
			Error  *string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Field+"/"+scenario.Input, func(t *testing.T) {
			actual, err := Normalize(scenario.Field, scenario.Input)
			if scenario.Error != nil {
				if err == nil || err.Error() != *scenario.Error {
					t.Fatalf("want error %q, got %q, %v", *scenario.Error, actual, err)
				}
			} else if err != nil || scenario.Output == nil || actual != *scenario.Output {
				t.Fatalf("want %q, got %q, %v", *scenario.Output, actual, err)
			}
		})
	}
}

func TestRegistryDefaultsAndSecretFlags(t *testing.T) {
	if len(definitions) != 20 {
		t.Fatal("managed setting inventory changed")
	}
	for _, definition := range definitions {
		if normalized, err := Normalize(definition.Field, definition.Default); err != nil || normalized != definition.Default {
			t.Fatalf("%s default is not canonical: %s %v", definition.Field, normalized, err)
		}
	}
}
