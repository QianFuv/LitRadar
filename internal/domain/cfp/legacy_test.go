package cfp

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

// TestReviewedLegacyNotices compares every normalized field and state against the existing Rust fixture.
func TestReviewedLegacyNotices(t *testing.T) {
	seedBytes, err := os.ReadFile("../../../assets/cfp/seed.json")
	if err != nil {
		t.Fatal(err)
	}
	fixtureBytes, err := os.ReadFile("../../../tests/migration/cfp/legacy.json")
	if err != nil {
		t.Fatal(err)
	}
	var seed Seed
	var fixture struct {
		Times    []string `json:"times"`
		Expected []struct {
			Id     string  `json:"id"`
			Digest string  `json:"digest"`
			States []State `json:"states"`
		} `json:"expected"`
	}
	if err = json.Unmarshal(seedBytes, &seed); err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(fixtureBytes, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(seed.Sources) != 1622 || len(fixture.Expected) != 1622 {
		t.Fatal("legacy coverage changed")
	}
	for index, source := range seed.Sources {
		expected := fixture.Expected[index]
		t.Run(expected.Id, func(t *testing.T) {
			notice := ParseSource(source)
			if notice == nil {
				t.Fatalf("rejected %+v", source)
			}
			encoded, err := jsonvalue.EncodeJson(notice)
			if err != nil {
				t.Fatal(err)
			}
			var canonical map[string]any
			if err = json.Unmarshal([]byte(encoded), &canonical); err != nil {
				t.Fatal(err)
			}
			encoded, err = jsonvalue.EncodeJson(canonical)
			if err != nil {
				t.Fatal(err)
			}
			digest := sha256.Sum256([]byte(encoded))
			if notice.Id != expected.Id || hex.EncodeToString(digest[:]) != expected.Digest {
				t.Fatalf("normalized mismatch: %s", encoded)
			}
			for position, instant := range fixture.Times {
				now, err := time.Parse(time.RFC3339, instant)
				if err != nil {
					t.Fatal(err)
				}
				if state := notice.State(now); state != expected.States[position] {
					t.Errorf("at %s got %s want %s", instant, state, expected.States[position])
				}
			}
		})
	}
}
