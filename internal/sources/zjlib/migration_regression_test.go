package zjlib

import (
	"github.com/QianFuv/LitRadar/internal/testkit/testlog"
	"testing"
)

func TestFulltextFailureLogsRetainWorkerScopeWithoutArticleContent(t *testing.T) {
	const sentinel = "private-article-sentinel"
	ctx, finish := testlog.Capture(t)
	client := NewClient(NewFixtureTransport(FulltextMismatch))
	if _, err := client.StartQrLogin(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := client.PollQrLogin(ctx, 180, 2); err != nil {
		t.Fatal(err)
	}
	if _, err := client.WarmUpFulltextSession(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := client.DownloadMatchingPdf(ctx, ArticleIdentity{Title: sentinel, Authors: sentinel, JournalTitle: sentinel}, 10); err == nil {
		t.Fatal("mismatch downloaded PDF")
	}
	events := finish()
	if len(testlog.Events(events, "source.fallback.activated")) == 0 {
		t.Fatal(events)
	}
	selected := testlog.Events(events, "source.fulltext.failed")
	if len(selected) != 1 {
		t.Fatal(events)
	}
	testlog.Require(t, selected[0], map[string]any{"provider": "zjlib", "error_kind": "no_exact_match"})
	span, ok := selected[0]["span"].(map[string]any)
	if !ok {
		t.Fatal(selected[0])
	}
	testlog.Require(t, span, map[string]any{"run_id": "run-source-correlation", "worker_id": float64(7)})
	testlog.Private(t, events, sentinel)
}
