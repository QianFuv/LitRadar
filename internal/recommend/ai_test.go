package recommend

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/transport"
)

type aiFixture struct {
	responses []json.RawMessage
	requests  []any
	after     func()
}

func (fixture *aiFixture) PostJson(ctx context.Context, location string, headers http.Header, body any, timeout time.Duration) (outbound.Response, error) {
	if _, err := sources.Json(body); err != nil {
		return outbound.Response{}, err
	}
	values := map[string]any{}
	for name := range headers {
		values[strings.ToLower(name)] = headers.Get(name)
	}
	fixture.requests = append(fixture.requests, map[string]any{"url": location, "headers": values, "body": body})
	if fixture.after != nil {
		fixture.after()
	}
	if len(fixture.responses) == 0 {
		return outbound.Response{}, errors.New("fixture exhausted")
	}
	var response struct {
		Error      string
		Status     int
		RequestId  *string `json:"request_id"`
		RetryAfter *uint64 `json:"retry_after"`
		Body       json.RawMessage
	}
	if err := json.Unmarshal(fixture.responses[0], &response); err != nil {
		return outbound.Response{}, err
	}
	fixture.responses = fixture.responses[1:]
	switch response.Error {
	case "connect_failed":
		return outbound.Response{}, outbound.ConnectFailed
	case "timeout":
		return outbound.Response{}, outbound.TimedOut
	case "transport":
		return outbound.Response{}, outbound.RequestFailed
	}
	if response.Status == 0 {
		response.Status = 200
	}
	var parsed any
	if len(response.Body) > 0 {
		var err error
		parsed, err = transport.ParseJson(response.Body)
		if err != nil {
			return outbound.Response{}, err
		}
	}
	return outbound.Response{StatusCode: response.Status, RequestId: response.RequestId, RetryAfterSeconds: response.RetryAfter, Body: parsed}, nil
}

func TestAiRechecksAllowlistBeforeEachAttempt(t *testing.T) {
	fixture := &aiFixture{responses: []json.RawMessage{json.RawMessage(`{"status":503,"retry_after":0}`)}, requests: []any{}}
	isAllowed := true
	fixture.after = func() { isAllowed = false }
	client := &AiClient{transport: fixture, allowedBaseUrls: func(context.Context) ([]string, error) {
		if isAllowed {
			return []string{"https://example.test/v1/"}, nil
		}
		return []string{}, nil
	}, retryAttempts: 2, timeout: time.Second, wait: func(context.Context, time.Duration) error { return nil }}
	_, err := client.SelectArticles(context.Background(), AiRuntimeConfig{BaseUrl: "https://example.test/v1/"}, domain.Subscriber{}, Defaults{}, nil)
	if err == nil || err.Error() != "AI endpoint is not approved" || len(fixture.requests) != 1 {
		t.Fatalf("revoked endpoint attempted: %v requests=%d", err, len(fixture.requests))
	}
}

func TestAiBudgetIsSharedAcrossFormatsAndCompletions(t *testing.T) {
	fixture := &aiFixture{responses: []json.RawMessage{json.RawMessage(`{"body":null}`), json.RawMessage(`{"body":{"choices":[{"message":{"parsed":{"selected":[]}}}]}}`)}, requests: []any{}}
	control := domain.NewExecutionControl(float64(time.Now().Unix()+60), 2, func() (bool, error) { return false, nil })
	client := &AiClient{transport: fixture, allowedBaseUrls: func(context.Context) ([]string, error) { return []string{"https://example.test/v1/"}, nil }, timeout: time.Second, control: control}
	config := AiRuntimeConfig{BaseUrl: "https://example.test/v1/"}
	if _, err := client.SelectArticles(context.Background(), config, domain.Subscriber{}, Defaults{}, nil); err != nil {
		t.Fatal(err)
	}
	_, err := client.SummarizeSelectedArticles(context.Background(), config, domain.Subscriber{}, []storage.ArticleCandidate{{ArticleId: 1}})
	if !errors.Is(err, domain.ControlBudgetExhausted) || len(fixture.requests) != 2 {
		t.Fatalf("budget bypass: %v requests=%d", err, len(fixture.requests))
	}
}
