package citation

import (
	"errors"

	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

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
