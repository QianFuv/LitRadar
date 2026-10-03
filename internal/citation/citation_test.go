package citation

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

func TestOriginalRustCitationBytesAndBoundaries(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "tests", "migration", "storage", "citation-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Format       string
			Articles     []domain.FavoriteCitation
			MaximumBytes int `json:"maximum_bytes"`
			Output       *string
			Limited      bool
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for index, scenario := range fixture.Cases {
		t.Run(scenario.Format+"/"+strconv.Itoa(index), func(t *testing.T) {
			serialize := map[string]func([]domain.FavoriteCitation, int) (string, error){"bibtex": Bibtex, "ris": Ris, "endnote": EndnoteXml}[scenario.Format]
			actual, err := serialize(scenario.Articles, scenario.MaximumBytes)
			if scenario.Limited {
				if !errors.Is(err, ErrOutputLimit) || actual != "" {
					t.Fatalf("partial or oversized output escaped: %q %v", actual, err)
				}
			} else if err != nil || scenario.Output == nil || actual != *scenario.Output {
				t.Fatalf("Rust mismatch: %q %v", actual, err)
			}
		})
	}
}

func TestEightMiBExportBoundary(t *testing.T) {
	const maximum = 8 * 1024 * 1024
	empty, err := Bibtex([]domain.FavoriteCitation{{}}, maximum)
	if err != nil {
		t.Fatal(err)
	}
	title := strings.Repeat("x", maximum-len(empty))
	article := []domain.FavoriteCitation{{Title: &title}}
	if output, err := Bibtex(article, maximum); err != nil || len(output) != maximum {
		t.Fatalf("exact bound failed: %d %v", len(output), err)
	}
	if output, err := Bibtex(article, maximum-1); !errors.Is(err, ErrOutputLimit) || output != "" {
		t.Fatal("oversized output escaped")
	}
}
