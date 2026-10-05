package recommend

import (
	"context"
	"encoding/json"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/testkit/testlog"
	"strings"
	"testing"
	"time"
)

func TestActualAiRetryLogsAndTerminalFailureRedactPromptAndCredentials(t *testing.T) {
	const sentinel = "private-ai-sentinel"
	ctx, finish := testlog.Capture(t)
	response := json.RawMessage(`{"status":503,"request_id":"request-123","retry_after":7,"body":{"error":"` + sentinel + `"}}`)
	fixture := &aiFixture{responses: []json.RawMessage{response, response, response}}
	config := AiRuntimeConfig{BaseUrl: "https://" + sentinel + ".example/v1", ApiKey: sentinel, Model: "fixture-model", SystemPrompt: sentinel}
	client := &AiClient{transport: fixture, allowedBaseUrls: func(context.Context) ([]string, error) { return []string{config.BaseUrl}, nil }, retryAttempts: 2, temperature: 0.2, timeout: time.Second, wait: func(context.Context, time.Duration) error { return nil }}
	_, err := client.SelectArticles(ctx, config, domain.Subscriber{Name: sentinel, Keywords: []string{sentinel}}, Defaults{MaxCandidates: 120}, []storage.ArticleCandidate{{ArticleId: 104, Title: sentinel, Abstract: sentinel}})
	if err == nil || strings.Contains(err.Error(), sentinel) {
		t.Fatal("missing or unsafe failure", err)
	}
	events := finish()
	failed := testlog.Events(events, "ai.request.failed")
	if len(failed) != 3 || len(fixture.requests) != 3 || len(testlog.Events(events, "ai.response_format.fallback")) != 0 || len(testlog.Events(events, "ai.completion.failed")) != 1 {
		t.Fatal(events)
	}
	for ordinal, event := range failed {
		testlog.Require(t, event, map[string]any{"attempt": float64(ordinal + 1), "http_status": float64(503), "will_retry": ordinal < 2, "response_format": "json_schema"})
		span, ok := event["span"].(map[string]any)
		if !ok {
			t.Fatal(event)
		}
		testlog.Require(t, span, map[string]any{"operation": "selection"})
	}
	testlog.Private(t, events, sentinel)
}
