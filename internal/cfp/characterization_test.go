package cfp

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
)

// TestDiscoveryAmbiguityRejectsWholePage protects complete-snapshot admission with a valid neighboring card.
func TestDiscoveryAmbiguityRejectsWholePage(t *testing.T) {
	config := SourceConfig{Adapter: SpringerCollections, CatalogIds: []string{"a"}, JournalTitle: "Journal", IdentityTexts: []string{"Journal"}, AllowedUrls: []UrlRule{{Host: "publisher.example", PathPrefix: "/"}}}
	document := Document{FinalUrl: "https://publisher.example/calls", Format: "html", Text: `<h1>Journal</h1><h2>Valid special issue</h2><p>Submission deadline: 31 December 2026</p><p>Submission status: Open for submissions</p><p>Research scope.</p>`}
	if parsed, err := ParsePage(config, document, "2026-10-07", true); err != nil || len(parsed.Sources) != 1 {
		t.Fatal("valid neighboring card rejected", parsed, err)
	}
	document.Text += `<h2>Ambiguous special issue</h2><p>Research scope.</p>`
	if _, err := ParsePage(config, document, "2026-10-07", true); !errors.Is(err, ErrUnrecognized) {
		t.Fatal("ambiguous neighbor admitted", err)
	}
}

// TestFirstTitleAnchorCannotBeReplacedByLaterValidAnchor protects first-match publisher link semantics.
func TestFirstTitleAnchorCannotBeReplacedByLaterValidAnchor(t *testing.T) {
	config := SourceConfig{Adapter: ElsevierCalls, CatalogIds: []string{"a"}, JournalTitle: "Journal", IdentityTexts: []string{"Journal"}, AllowedUrls: []UrlRule{{Host: "publisher.example", PathPrefix: "/"}}}
	document := Document{FinalUrl: "https://publisher.example/calls", Format: "html", Text: `<a href="https://disallowed.example/topic">Topic</a><a href="/topic">Topic</a><h1>Journal</h1><h2>Call for papers</h2><h3>Topic</h3>`}
	if _, err := ParsePage(config, document, "2026-10-07", true); !errors.Is(err, ErrUnrecognized) {
		t.Fatal("later anchor replaced first", err)
	}
}

// TestOriginalNavigationLimitPrecedesDeduplication protects the three encountered-navigation budget.
func TestOriginalNavigationLimitPrecedesDeduplication(t *testing.T) {
	document := Document{FinalUrl: "https://publisher.example/start", Format: "html", Text: `<a href="/topic">Topic</a><a rel="next" href="/next">next</a><a rel="next" href="/next">next</a><a rel="next" href="/next">next</a><a rel="next" href="/fourth">next</a>`}
	links := OriginalLinks(domain.Source{Title: "Topic"}, document)
	if !reflect.DeepEqual(links, []string{"https://publisher.example/topic", "https://publisher.example/next"}) {
		t.Fatal("navigation admission changed", links)
	}
}

// TestPdfTitleRuneBoundary protects the inclusive 600-rune title position for original PDF extraction.
func TestPdfTitleRuneBoundary(t *testing.T) {
	for _, scenario := range []struct {
		prefix   int
		expected error
	}{{600, nil}, {601, ErrUnrecognized}} {
		document := Document{FinalUrl: "https://publisher.example/original.pdf", Format: "pdf_text", Text: strings.Repeat("界", scenario.prefix) + "Topic\nOriginal scope"}
		body, err := pdfBody(domain.Source{Title: "Topic"}, document, func(string, string, string) bool { return false })
		if !errors.Is(err, scenario.expected) || err == nil && body != "Original scope" {
			t.Fatal("PDF title boundary changed", scenario.prefix, body, err)
		}
	}
}

// TestCachedCapturePrecedesExpiredAdmission protects successful raw-key cache hits from later cancellation.
func TestCachedCapturePrecedesExpiredAdmission(t *testing.T) {
	cache := newCaptureCache()
	document := Document{FinalUrl: "https://publisher.example/original", Text: "original", Format: "html"}
	cache.documents["raw key"] = capturedDocument{document: document}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	actual, err := cache.capture(ctx, SourceConfig{}, "raw key", RefreshOptions{}, time.Time{}, false)
	if err != nil || actual != document || cache.bytes != 0 {
		t.Fatal("cached original reacquired", actual, err, cache.bytes)
	}
}

// TestResumeDuplicateBudgetFailureKeepsEarlierAliases protects cumulative bytes and partial restore state.
func TestResumeDuplicateBudgetFailureKeepsEarlierAliases(t *testing.T) {
	entry := map[string]string{"requestedUrl": "requested", "url": "https://link.springer.com/collections/a", "text": "abc", "format": "html"}
	payload, err := json.Marshal(map[string]any{"result": map[string]string{"sourceKey": "journal:a"}, "documents": []any{entry, entry}})
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "capture.json")
	if err := os.WriteFile(path, payload, 0600); err != nil {
		t.Fatal(err)
	}
	cache := newCaptureCache()
	cache.bytes = MaxCaptureBytes - 5
	if err := cache.resume(path, "journal:a"); err == nil || err.Error() != "Saved capture exceeds the source budget" {
		t.Fatal("duplicate restore stopped counting", err)
	}
	if cache.bytes != MaxCaptureBytes+1 || len(cache.documents) != 2 || cache.documents["requested"].document.Text != "abc" || !cache.browserAttempts["requested"] {
		t.Fatal("partial restore lost earlier state", cache.bytes, cache.documents, cache.browserAttempts)
	}
}
