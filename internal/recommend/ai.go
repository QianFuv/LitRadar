package recommend

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// AiError exposes only fixed classifications and safe upstream metadata.
type AiError struct {
	Kind              string
	StatusCode        int
	RequestId         *string
	RetryAfterSeconds *uint64
	message           string
	control           error
}

func (err *AiError) Error() string {
	switch err.Kind {
	case "connect_failed":
		return "AI endpoint connection failed"
	case "timeout":
		return "AI request timed out"
	case "http_status":
		message := fmt.Sprintf("AI request failed with HTTP %d", err.StatusCode)
		if err.RequestId != nil {
			message += " (request ID: " + *err.RequestId + ")"
		}
		return message
	default:
		return err.message
	}
}

// Unwrap preserves execution-control classification without retaining transport secrets.
func (err *AiError) Unwrap() error { return err.control }

// AiClient performs controlled requests against a freshly read administrator allowlist.
type AiClient struct {
	transport       outbound.JsonTransport
	allowedBaseUrls func(context.Context) ([]string, error)
	retryAttempts   int
	temperature     float64
	timeout         time.Duration
	control         *domain.ExecutionControl
	wait            func(context.Context, time.Duration) error
}

// NewAiClient creates the production client using request-time settings and a shared durable job control.
func NewAiClient(repository *settings.Repository, retryAttempts int, temperature float64, timeout time.Duration, control *domain.ExecutionControl) *AiClient {
	timeout = max(time.Second, timeout)
	return &AiClient{outbound.New(timeout), repository.AiBaseUrls, min(max(retryAttempts, 0), 10), temperature, timeout, control, outbound.Wait}
}

// SelectArticles obtains and orders model choices without performing local selection or side effects.
func (client *AiClient) SelectArticles(ctx context.Context, config AiRuntimeConfig, subscriber domain.Subscriber, defaults Defaults, candidates []storage.ArticleCandidate) (domain.SelectionResult, error) {
	payload, schema := selectionRequest(subscriber, defaults, candidates)
	response, err := client.complete(ctx, config, "paper_selection", schema, selectionSystemPrompt(config), payload, SelectionPayload)
	if err != nil {
		return domain.SelectionResult{}, err
	}
	result := domain.SelectionResult{Selections: []domain.RankedSelection{}}
	result.Summary, _ = response["summary"].(string)
	items, _ := response["selected"].([]any)
	for _, item := range items {
		object, _ := item.(map[string]any)
		if articleId, ok := jsonInt64(object["article_id"]); ok {
			score, _ := jsonFloat64(object["score"])
			result.Selections = append(result.Selections, domain.RankedSelection{ArticleId: articleId, Score: score})
		}
	}
	sort.SliceStable(result.Selections, func(first, second int) bool { return result.Selections[first].Score > result.Selections[second].Score })
	return result, nil
}

// SummarizeSelectedArticles skips empty input and summarizes only accepted candidates.
func (client *AiClient) SummarizeSelectedArticles(ctx context.Context, config AiRuntimeConfig, subscriber domain.Subscriber, candidates []storage.ArticleCandidate) (string, error) {
	if len(candidates) == 0 {
		return "", nil
	}
	payload, schema := summaryRequest(subscriber, candidates)
	response, err := client.complete(ctx, config, "selected_paper_summary", schema, summarySystemPrompt(config), payload, SummaryPayload)
	if err != nil {
		return "", err
	}
	summary, _ := response["summary"].(string)
	return strings.TrimSpace(summary), nil
}

func (client *AiClient) complete(ctx context.Context, config AiRuntimeConfig, name string, schema any, system string, payload any, kind PayloadKind) (result map[string]any, resultError error) {
	started := time.Now()
	logger := slog.Default().With("component", "delivery", "provider", "openai_compatible", "endpoint", "chat_completions", "operation", string(kind))
	logger.Info("ai.completion.started", "event", "ai.completion.started", "outcome", "started")
	defer func() {
		if resultError != nil {
			logger.Warn("ai.completion.failed", "event", "ai.completion.failed", "outcome", "failure", "error_kind", aiErrorKind(resultError), "duration_ms", time.Since(started).Milliseconds())
		} else {
			logger.Info("ai.completion.completed", "event", "ai.completion.completed", "outcome", "success", "duration_ms", time.Since(started).Milliseconds())
		}
	}()
	location, err := completionUrl(config.BaseUrl)
	if err != nil {
		return nil, err
	}
	encoded, err := sources.Json(payload)
	if err != nil {
		return nil, &AiError{Kind: "transport", message: string(outbound.RequestFailed)}
	}
	formats := []string{"json_schema", "json_object", "plain_json"}
	if location.Hostname() == "api.deepseek.com" {
		formats = formats[1:]
	}
	for formatIndex, format := range formats {
		if formatIndex > 0 {
			logger.Warn("ai.response_format.fallback", "event", "ai.response_format.fallback", "outcome", "fallback", "from_format", formats[formatIndex-1], "to_format", format)
		}
		for attempt := 0; attempt <= client.retryAttempts; attempt++ {
			timeout := client.timeout
			if client.control != nil {
				timeout, err = client.control.BeginAiRequest(timeout)
				if err != nil {
					return nil, controlAiError(err)
				}
			}
			body := map[string]any{"model": config.Model, "temperature": client.temperature, "messages": []any{map[string]any{"role": "system", "content": system}, map[string]any{"role": "user", "content": string(encoded)}}}
			switch format {
			case "json_schema":
				body["response_format"] = map[string]any{"type": format, "json_schema": map[string]any{"name": name, "strict": true, "schema": schema}}
			case "json_object":
				body["response_format"] = map[string]any{"type": format}
			}
			attemptStarted := time.Now()
			response, requestError := client.send(ctx, location.Href(false), config.ApiKey, body, timeout)
			if requestError == nil {
				result, err = ExtractResponsePayload(response.Body, kind)
				if err != nil {
					requestError = &AiError{Kind: "invalid_response", message: err.Error()}
				}
			}
			if requestError == nil {
				logger.Info("ai.request.completed", "event", "ai.request.completed", "outcome", "success", "response_format", format, "attempt", attempt+1, "http_status", response.StatusCode, "duration_ms", time.Since(attemptStarted).Milliseconds())
				return result, nil
			}
			isRetryable := requestError.Kind == "connect_failed" || requestError.Kind == "timeout" || requestError.Kind == "http_status" && (requestError.StatusCode == 429 || requestError.StatusCode == 502 || requestError.StatusCode == 503 || requestError.StatusCode == 504)
			willRetry := attempt < client.retryAttempts && isRetryable
			willFallback := !willRetry && requestError.Kind == "invalid_response" && formatIndex+1 < len(formats)
			attributes := []any{"event", "ai.request.failed", "outcome", "failure", "response_format", format, "attempt", attempt + 1, "error_kind", requestError.Kind, "will_retry", willRetry, "will_fallback", willFallback, "duration_ms", time.Since(attemptStarted).Milliseconds()}
			if requestError.Kind == "http_status" {
				attributes = append(attributes, "http_status", requestError.StatusCode)
			}
			logger.Warn("ai.request.failed", attributes...)
			if willRetry {
				delay := outbound.RetryDelay(attempt, requestError.RetryAfterSeconds)
				if client.control != nil {
					err = client.control.Wait(ctx, delay)
				} else {
					err = client.wait(ctx, delay)
				}
				if err != nil {
					return nil, controlAiError(err)
				}
				continue
			}
			if !willFallback {
				return nil, requestError
			}
			break
		}
	}
	return nil, &AiError{Kind: "invalid_response", message: "AI request was not attempted"}
}

func completionUrl(base string) (*whatwg.Url, error) {
	location, err := whatwg.NewParser().ParseRef(base, "chat/completions")
	if err != nil {
		return nil, &AiError{Kind: "transport", message: "AI endpoint URL is invalid"}
	}
	return location, nil
}

func (client *AiClient) send(ctx context.Context, location, apiKey string, body any, timeout time.Duration) (outbound.Response, *AiError) {
	allowed, err := client.allowedBaseUrls(ctx)
	isAllowed := false
	if err == nil {
		for _, base := range allowed {
			candidate, err := completionUrl(base)
			if err == nil && candidate.Href(false) == location {
				isAllowed = true
				break
			}
		}
	}
	if !isAllowed {
		return outbound.Response{}, &AiError{Kind: "transport", message: "AI endpoint is not approved"}
	}
	headers := http.Header{"Authorization": {"Bearer " + apiKey}, "Content-Type": {"application/json"}, "Http-Referer": {"https://github.com/openai/codex"}, "X-Title": {"LitRadar"}}
	response, err := client.transport.PostJson(ctx, location, headers, body, timeout)
	if err != nil {
		kind, message := "transport", string(outbound.RequestFailed)
		var transportError outbound.Error
		if errors.As(err, &transportError) {
			message = transportError.Error()
		}
		if errors.Is(err, outbound.ConnectFailed) {
			kind = "connect_failed"
		} else if errors.Is(err, outbound.TimedOut) {
			kind = "timeout"
		}
		return outbound.Response{}, &AiError{Kind: kind, message: message}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return response, &AiError{Kind: "http_status", StatusCode: response.StatusCode, RequestId: response.RequestId, RetryAfterSeconds: response.RetryAfterSeconds}
	}
	return response, nil
}

func controlAiError(err error) *AiError {
	var control domain.ControlError
	if !errors.As(err, &control) {
		control = domain.ControlCancelled
	}
	return &AiError{Kind: string(control), message: control.Error(), control: control}
}

func aiErrorKind(err error) string {
	var failure *AiError
	if errors.As(err, &failure) {
		return failure.Kind
	}
	return "transport"
}
