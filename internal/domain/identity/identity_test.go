package identity

import (
	"encoding/json"
	"os"
	"testing"
)

func TestFrozenRustIdentityVectors(t *testing.T) {
	bytes, err := os.ReadFile("../../../tests/data/migration/rust/identifiers.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Article Id `json:"article"`
		Journal Id `json:"journal"`
		Stable  []struct {
			Input, Prefix string
			Expected      Id `json:"expectedDecimal"`
		} `json:"stable"`
	}
	if err := json.Unmarshal(bytes, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.Article != 9223372036854775807 || fixture.Journal != 9007199254740993 {
		t.Fatal("Identifier precision lost")
	}
	for _, vector := range fixture.Stable {
		if actual := Stable(vector.Input, vector.Prefix); actual != vector.Expected {
			t.Errorf("Stable(%q): %d != %d", vector.Input, actual, vector.Expected)
		}
	}
	for _, invalid := range []string{`null`, `true`, `1.0`, `1e3`, `"9223372036854775808"`, `[]`} {
		var value Id
		if err := json.Unmarshal([]byte(invalid), &value); err == nil {
			t.Errorf("Accepted invalid ID %s", invalid)
		}
	}
	encoded, err := json.Marshal(fixture.Article)
	if err != nil || string(encoded) != `"9223372036854775807"` {
		t.Fatalf("Public wire precision changed: %s %v", encoded, err)
	}
}
