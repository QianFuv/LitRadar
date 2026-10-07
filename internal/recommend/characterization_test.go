package recommend

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"reflect"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/testkit/testlog"
)

func TestManifestKeepsNumericSpellingOrderAndUnknownValueLeniency(t *testing.T) {
	raw := []byte(`{"unknown":"\ud800","changed_issue_keys":["1:2","01:2","1:2"],"changed_inpress_journal_ids":["+02",2,1.0],"notifiable_article_ids":["+02",2,"01",1],"run_id":" run "}`)
	value, err := ParseChangeManifest(raw, "fixture.sqlite")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(value.PendingIssueKeys, []string{"1:2", "01:2", "1:2"}) || !reflect.DeepEqual(value.PendingInpressKeys, []string{"2"}) || !reflect.DeepEqual(value.PendingArticleIds, []int64{2, 1}) || value.RunId == nil || *value.RunId != "run" {
		t.Fatal("manifest spelling/coercion order changed", value)
	}
}

func TestSnapshotRejectsEarlierInvalidDuplicateEvenWhenLaterValueIsValid(t *testing.T) {
	for _, raw := range []string{`{"issue_article_counts":{"1:2":1.0,"1:2":1}}`, `{"issue_article_counts":{"1:2":"bad","1:2":1}}`, `[{"1:2":null,"1:2":1}]`} {
		value, err := ParseSnapshot([]byte(raw))
		if err == nil || !reflect.DeepEqual(value, Snapshot{}) {
			t.Fatal("earlier invalid duplicate hidden", value, err)
		}
	}
	value, err := ParseSnapshot([]byte(`{"issue_article_counts":{"1:2":1,"1:2":2}}`))
	if err != nil || value.IssueArticleCounts["1:2"] != 2 {
		t.Fatal("valid duplicate did not keep last count", value, err)
	}
}

func TestResponseParsedPresenceAndRefusalSuppressValidContent(t *testing.T) {
	message := map[string]any{"parsed": nil, "content": `{"selected":[]}`}
	response := map[string]any{"choices": []any{map[string]any{"message": message}}}
	if _, err := ExtractResponsePayload(response, SelectionPayload); !errors.Is(err, errStructuredPayload) {
		t.Fatal("parsed null fell back to content", err)
	}
	message["refusal"] = " refused "
	if _, err := ExtractResponsePayload(response, SelectionPayload); err == nil || err.Error() != "AI model refused structured output" {
		t.Fatal("refusal priority changed", err)
	}
}

func TestAiBudgetRejectionEmitsNoRequestFailure(t *testing.T) {
	ctx, finish := testlog.Capture(t)
	fixture := &aiFixture{}
	control := domain.NewExecutionControl(float64(time.Now().Unix()+60), 0, func() (bool, error) { return false, nil })
	client := &AiClient{transport: fixture, control: control, timeout: time.Second}
	_, err := client.complete(ctx, AiRuntimeConfig{BaseUrl: "https://example.test/v1/"}, "fixture", nil, "", nil, SelectionPayload)
	events := finish()
	if !errors.Is(err, domain.ControlBudgetExhausted) || len(fixture.requests) != 0 {
		t.Fatal("budget admission changed", err)
	}
	if len(testlog.Events(events, "ai.request.failed")) != 0 || len(testlog.Events(events, "ai.completion.failed")) != 1 {
		t.Fatal("unstarted request emitted failure", events)
	}
}

func TestAiRetryCounterRestartsAfterResponseFormatFallback(t *testing.T) {
	ctx, finish := testlog.Capture(t)
	fixture := &aiFixture{responses: []json.RawMessage{json.RawMessage(`{"body":null}`), json.RawMessage(`{"status":503}`), json.RawMessage(`{"body":{"choices":[{"message":{"parsed":{"selected":[]}}}]}}`)}}
	config := AiRuntimeConfig{BaseUrl: "https://example.test/v1/"}
	client := &AiClient{transport: fixture, allowedBaseUrls: func(context.Context) ([]string, error) { return []string{config.BaseUrl}, nil }, retryAttempts: 1, timeout: time.Second, wait: func(context.Context, time.Duration) error { return nil }}
	_, err := client.complete(ctx, config, "fixture", nil, "", nil, SelectionPayload)
	events := finish()
	if err != nil || len(fixture.requests) != 3 {
		t.Fatal("format retry sequence changed", err)
	}
	failures := testlog.Events(events, "ai.request.failed")
	if len(failures) != 2 || len(testlog.Events(events, "ai.response_format.fallback")) != 1 {
		t.Fatal(events)
	}
	testlog.Require(t, failures[0], map[string]any{"attempt": float64(1), "response_format": "json_schema", "will_fallback": true, "will_retry": false})
	testlog.Require(t, failures[1], map[string]any{"attempt": float64(1), "response_format": "json_object", "will_fallback": false, "will_retry": true})
}

func TestSelectionRoundNaNReplacementKeepsSourceSliceOwnership(t *testing.T) {
	client := &scriptedSelection{selections: []domain.SelectionResult{{Selections: []domain.RankedSelection{{ArticleId: 1, Score: math.NaN()}}}, {Selections: []domain.RankedSelection{{ArticleId: 1, Score: 3}}}}}
	_, request, _ := selectorFixture(client)
	original := append([]storage.ArticleCandidate(nil), request.CandidatesForModel...)
	result, err := selectRounds(context.Background(), client, AiRuntimeConfig{}, request, 2)
	if err != nil || len(result.Selections) != 1 || result.Selections[0].Score != 3 {
		t.Fatal("NaN replacement changed", result, err)
	}
	if !reflect.DeepEqual(request.CandidatesForModel, original) {
		t.Fatal("round filtering changed caller slice")
	}
}

func TestMarkdownHeaderFallbackKeepsRuneBudget(t *testing.T) {
	content := BuildMarkdownContent("db", "run", domain.Subscriber{Name: strings.Repeat("中", MaxPushContentLength)}, "", nil, nil)
	if utf8.RuneCountInString(content) != MaxPushContentLength || len(content) <= MaxPushContentLength {
		t.Fatal("header rune fallback changed", len(content))
	}
}

func TestMarkdownMissingPositionsAndOversizeSectionsRetainOrdering(t *testing.T) {
	selections := make([]domain.RankedSelection, 21)
	selections[20].ArticleId = 1
	candidates := map[int64]storage.ArticleCandidate{1: {ArticleId: 1, Title: "visible"}}
	content := BuildMarkdownContent("db", "run", domain.Subscriber{}, "", selections, candidates)
	if strings.Contains(content, "visible") {
		t.Fatal("selection position cap changed")
	}
	candidates[2] = storage.ArticleCandidate{ArticleId: 2, Title: strings.Repeat("x", MaxPushContentLength)}
	content = BuildMarkdownContent("db", "run", domain.Subscriber{}, "", []domain.RankedSelection{{ArticleId: 2}, {ArticleId: 1}}, candidates)
	if !strings.Contains(content, "### 2. visible") || !strings.Contains(content, "Selected Articles: 1") {
		t.Fatal("whole section skipping/numbering changed")
	}
}
