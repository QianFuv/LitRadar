package recommend

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func compareJson(t *testing.T, actual, expected []byte) {
	t.Helper()
	var left, right any
	for _, target := range []struct {
		data  []byte
		value *any
	}{{actual, &left}, {expected, &right}} {
		decoder := json.NewDecoder(bytes.NewReader(target.data))
		decoder.UseNumber()
		if err := decoder.Decode(target.value); err != nil {
			t.Fatal(err)
		}
	}
	if !equalObservation(left, right) {
		t.Fatalf("client differs from original\nactual: %s\nexpected: %s", actual, expected)
	}
}

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

func TestOriginalAiClientObservations(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/delivery/client-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct{ Name, Input, Output string }
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, vector := range corpus.Cases {
		var input struct {
			Op         string
			Config     AiRuntimeConfig
			Subscriber domain.Subscriber
			Candidates []storage.ArticleCandidate
			Responses  []json.RawMessage
			Retries    int
		}
		if err := json.Unmarshal([]byte(vector.Input), &input); err != nil {
			t.Fatal(err)
		}
		if input.Op == "pushplus" {
			continue
		}
		t.Run(vector.Name, func(t *testing.T) {
			fixture := &aiFixture{responses: input.Responses, requests: []any{}}
			client := &AiClient{transport: fixture, allowedBaseUrls: func(context.Context) ([]string, error) { return []string{input.Config.BaseUrl}, nil }, retryAttempts: min(input.Retries, 10), temperature: 0.2, timeout: time.Second, wait: func(context.Context, time.Duration) error { return nil }}
			var value any
			var resultError error
			if input.Op == "summary" {
				value, resultError = client.SummarizeSelectedArticles(context.Background(), input.Config, input.Subscriber, input.Candidates)
			} else {
				value, resultError = client.SelectArticles(context.Background(), input.Config, input.Subscriber, Defaults{MaxCandidates: 120}, input.Candidates)
			}
			result := map[string]any{"value": value}
			if resultError != nil {
				result = map[string]any{"error": resultError.Error()}
			}
			actual, err := json.Marshal(map[string]any{"result": result, "requests": fixture.requests})
			if err != nil {
				t.Fatal(err)
			}
			compareJson(t, actual, []byte(vector.Output))
		})
	}
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
