package scholarly

import (
	"io"
	"net/http"
	"testing"

	"github.com/QianFuv/LitRadar/internal/testkit/testlog"
)

func TestActualSourceFailureLogsRetainCorrelationWithoutRequestSecrets(t *testing.T) {
	const sentinel = "source-private-sentinel"
	ctx, finish := testlog.Capture(t)
	live, server := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Content-Type", "application/json")
		response.WriteHeader(400)
		io.WriteString(response, `{"error":"`+sentinel+`"}`)
	}, 1, 1)
	live.config.CrossrefMailtos = []string{sentinel}
	if _, err := live.Request(ctx, Request{Service: Crossref, Endpoint: "journal_works", Issn: sentinel}); err == nil {
		t.Fatal("Crossref failure lost")
	}
	if _, err := NewClient(live, true).FetchSemanticScholarByDois(ctx, []string{"10.1234/" + sentinel}, 100); err == nil {
		t.Fatal("Semantic Scholar failure lost")
	}
	events := finish()
	for name, fields := range map[string]map[string]any{
		"source.request.failed":           {"provider": "crossref", "endpoint": "journal_works", "attempt": float64(1), "http_status": float64(400), "will_retry": false},
		"source.semantic_scholar.attempt": {"provider": "semantic_scholar", "endpoint": "paper_batch", "method": "POST", "attempt": float64(1), "http_status": float64(400), "has_http_status": true, "will_retry": false, "outcome": "failure"},
	} {
		selected := testlog.Events(events, name)
		if len(selected) != 1 {
			t.Fatalf("%s: %v", name, events)
		}
		testlog.Require(t, selected[0], fields)
		span, ok := selected[0]["span"].(map[string]any)
		if !ok {
			t.Fatal(selected[0])
		}
		testlog.Require(t, span, map[string]any{"run_id": "run-source-correlation", "worker_id": float64(7)})
	}
	testlog.Private(t, events, sentinel, server.URL, "s2-secret-0")
}

func TestOpenalexFallbackLogsOnlySymbolicReason(t *testing.T) {
	ctx, finish := testlog.Capture(t)
	fixture := NewFixtureTransport(FixtureData{OpenAlexSourceWorkPages: [][]any{{map[string]any{"id": "W1"}}}, IsOpenAlexSourceWorksPlanRestricted: true})
	date := "2026-01-01"
	page, err := NewClient(fixture, true).FetchOpenAlexWorksBySourcePage(ctx, "private-source-sentinel", &date, nil)
	if err != nil || !page.DidFallbackToUnfiltered {
		t.Fatal(page, err)
	}
	events := finish()
	selected := testlog.Events(events, "source.fallback.activated")
	if len(selected) != 1 {
		t.Fatal(events)
	}
	testlog.Require(t, selected[0], map[string]any{"provider": "openalex", "fallback": "full_source_pages"})
	testlog.Private(t, events, "private-source-sentinel")
}
