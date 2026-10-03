package scholarly

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func (live *LiveTransport) buildRequest(ctx context.Context, deadline time.Time, method, url string, query []QueryPair, body any, key *string) (*http.Request, context.CancelFunc, error) {
	remaining, hasTime := transport.Remaining(deadline)
	if !hasTime || ctx.Err() != nil {
		return nil, func() {}, context.DeadlineExceeded
	}
	location, err := whatwg.NewParser().Parse(url)
	if err != nil {
		return nil, func() {}, err
	}
	separator := "?"
	if strings.Contains(location.Href(true), "?") {
		separator = "&"
	}
	address := location.Href(true) + separator + EncodeQuery(query)
	var encoded []byte
	if body != nil {
		encoded, err = domain.Json(body)
		if err != nil {
			return nil, func() {}, err
		}
	}
	if key != nil {
		for _, character := range []byte(*key) {
			if character != '\t' && (character < 32 || character == 127) {
				return nil, func() {}, errors.New("invalid request header")
			}
		}
	}
	attemptCtx, cancel := context.WithTimeout(ctx, min(seconds(max(live.config.TimeoutSeconds, 1)).timer(), remaining))
	request, err := http.NewRequestWithContext(attemptCtx, method, address, bytes.NewReader(encoded))
	if err != nil {
		cancel()
		return nil, func() {}, err
	}
	request.Header.Set("User-Agent", "LitRadar/0.1")
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	if key != nil {
		request.Header.Set("x-api-key", *key)
	}
	return request, cancel, nil
}
func responseJson(response *http.Response) (any, error) {
	body, err := transport.BoundedBytes(response, scholarlyMaximumBytes)
	if err != nil {
		return nil, err
	}
	return transport.ParseJson(body)
}
func retryHeader(headers http.Header) *scheduleTime {
	delay, ok := transport.RetryAfter(headers)
	if !ok {
		return nil
	}
	value := scheduleTime{delay.Seconds, delay.Nanoseconds}
	return &value
}
func unsignedHeader(headers http.Header, name string) *uint64 {
	value := headers.Get(name)
	for _, character := range []byte(value) {
		if character != '\t' && (character < 32 || character > 126) {
			return nil
		}
	}
	value = strings.TrimSpace(value)
	value = strings.TrimPrefix(value, "+")
	parsed, err := strconv.ParseUint(value, 10, 64)
	if err != nil {
		return nil
	}
	return &parsed
}
func openAlexHeaders(headers http.Header) rateHeaders {
	result := rateHeaders{Remaining: unsignedHeader(headers, "x-ratelimit-remaining"), CreditsUsed: unsignedHeader(headers, "x-ratelimit-credits-used"), RetryAfter: retryHeader(headers)}
	if reset := unsignedHeader(headers, "x-ratelimit-reset"); reset != nil {
		delay := seconds(*reset)
		result.ResetAfter = &delay
	}
	return result
}
func httpHealth(status uint16) health {
	switch status {
	case 401, 403:
		return healthAuthentication
	case 429:
		return healthRateLimited
	case 500, 502, 503, 504:
		return healthTransient
	default:
		return healthTerminal
	}
}
func asciiLower(value string) string {
	var output strings.Builder
	for index := range len(value) {
		character := value[index]
		if character >= 'A' && character <= 'Z' {
			character += 'a' - 'A'
		}
		output.WriteByte(character)
	}
	return output.String()
}
func openAlexHealth(status uint16, payload any) health {
	message, _ := field(payload, "message").(string)
	if status == 429 && strings.HasPrefix(asciiLower(strings.TrimSpace(message)), "insufficient budget") {
		return healthDailyLimited
	}
	return httpHealth(status)
}
func safeOpenAlexBody(endpoint string, payload any, query []QueryPair) any {
	if endpoint == "source_works" {
		safe := map[string]any{}
		for _, name := range []string{"error", "message"} {
			if text, ok := field(payload, name).(string); ok {
				for _, pair := range query {
					if pair.Value != "" {
						text = strings.ReplaceAll(text, pair.Value, "[REDACTED]")
					}
				}
				safe[name] = text
			}
		}
		if len(safe) > 0 {
			return safe
		}
	}
	return map[string]any{"error": "OpenAlex request failed"}
}
func safeSemanticBody(status uint16, payload any) any {
	failure := &Error{Kind: "HttpStatus", Service: SemanticScholar, Endpoint: "paper_batch", StatusCode: status, Body: payload}
	if isNoValidIds(failure) {
		return map[string]any{"error": "No valid paper ids given"}
	}
	return map[string]any{"error": "Semantic Scholar request failed"}
}

func (live *LiveTransport) executeOpenAlex(ctx context.Context, endpoint, url string, baseQuery []QueryPair) sourceExecution {
	deadline := live.logicalDeadline(ctx)
	maximum := max(len(live.config.OpenAlexApiKeys), 1) + 2
	excluded := []int{}
	execution := sourceExecution{attempts: []executionAttempt{}}
	for index := 0; index < maximum; index++ {
		lease, err := live.openAlex.reserve(ctx, excluded, deadline)
		if err != nil {
			if execution.err == nil {
				execution.err = err
			}
			return execution
		}
		result := live.openAlexAttempt(ctx, lease, deadline, endpoint, url, baseQuery, index+1, maximum)
		execution.attempts = append(execution.attempts, result.execution.attempts...)
		execution.payload = result.execution.payload
		execution.err = result.execution.err
		if !result.shouldRetry {
			return execution
		}
		excluded = append(excluded, lease.slot.Slot)
	}
	return execution
}

type attemptResult struct {
	execution   sourceExecution
	shouldRetry bool
}

func (live *LiveTransport) openAlexAttempt(ctx context.Context, lease *openAlexLease, deadline time.Time, endpoint, url string, baseQuery []QueryPair, number, maximum int) attemptResult {
	defer lease.cancel()
	result := attemptResult{}
	delay := seconds(1 << min(number-1, 5))
	isSearch := endpoint == "source_search"
	slot := lease.slot.Slot
	query := append(append([]QueryPair{}, baseQuery...), QueryPair{"api_key", live.config.OpenAlexApiKeys[slot]})
	if _, hasTime := transport.Remaining(deadline); !hasTime || ctx.Err() != nil {
		result.execution.err = sourceDeadlineError(OpenAlex)
		return result
	}
	request, cancel, err := live.buildRequest(ctx, deadline, "GET", url, query, nil, nil)
	if err != nil {
		lease.finish(rateHeaders{}, healthTerminal, scheduleTime{}, isSearch)
		result.execution.err = &Error{Kind: "Request", Service: OpenAlex, Endpoint: endpoint, Message: "request build failed"}
		return result
	}
	defer cancel()
	for _, pair := range baseQuery {
		if pair.Name == "filter" && strings.HasPrefix(asciiLower(strings.TrimLeftFunc(pair.Value, func(character rune) bool { return strings.TrimSpace(string(character)) == "" })), "doi:") && len(request.URL.String()) > 1900 {
			lease.finish(rateHeaders{}, healthTerminal, scheduleTime{}, isSearch)
			result.execution.err = &Error{Kind: "Configuration", Message: "OpenAlex DOI enrichment request exceeds the URL budget."}
			return result
		}
	}
	started := time.Now()
	response, err := live.client.Transport.RoundTrip(request)
	if err != nil {
		result.shouldRetry = number < maximum
		lease.finish(rateHeaders{}, healthTransient, delay, isSearch)
		result.execution.attempts = []executionAttempt{newAttempt(OpenAlex, endpoint, "GET", request.URL.String(), number, slot, nil, false, result.shouldRetry, "transport", started)}
		result.execution.err = &Error{Kind: "Request", Service: OpenAlex, Endpoint: endpoint, Message: "transport failure"}
		return result
	}
	status := uint16(response.StatusCode)
	headers := openAlexHeaders(response.Header)
	if status == 429 {
		throttle := delay
		if headers.RetryAfter != nil {
			throttle = later(throttle, *headers.RetryAfter)
		}
		lease.observeThrottle(throttle)
		headers.RetryAfter = nil
	}
	payload, bodyError := responseJson(response)
	if errors.Is(bodyError, transport.ErrInvalidJson) {
		payload = map[string]any{"error": "OpenAlex returned invalid JSON"}
		bodyError = nil
	}
	if bodyError != nil {
		isTooLarge := errors.Is(bodyError, transport.ErrTooLarge)
		outcome := healthTransient
		if status == 429 {
			outcome = healthRateLimited
		} else if isTooLarge {
			outcome = healthTerminal
		}
		result.shouldRetry = !isTooLarge && number < maximum
		lease.finish(headers, outcome, delay, isSearch)
		errorKind := "response_body"
		if isTooLarge {
			errorKind = "response_too_large"
		}
		result.execution.attempts = []executionAttempt{newAttempt(OpenAlex, endpoint, "GET", request.URL.String(), number, slot, &status, false, result.shouldRetry, errorKind, started)}
		if status == 429 {
			result.execution.err = &Error{Kind: "HttpStatus", Service: OpenAlex, Endpoint: endpoint, StatusCode: status, Body: map[string]any{"error": "OpenAlex request failed"}}
		} else {
			result.execution.err = &Error{Kind: "Request", Service: OpenAlex, Endpoint: endpoint, Message: bodyError.Error()}
		}
		return result
	}
	if status >= 200 && status < 300 {
		lease.finish(headers, healthSuccess, scheduleTime{}, isSearch)
		result.execution.payload = payload
		result.execution.attempts = []executionAttempt{newAttempt(OpenAlex, endpoint, "GET", request.URL.String(), number, slot, &status, true, false, "none", started)}
		return result
	}
	outcome := openAlexHealth(status, payload)
	result.shouldRetry = outcome != healthTerminal && number < maximum
	lease.finish(headers, outcome, delay, isSearch)
	result.execution.attempts = []executionAttempt{newAttempt(OpenAlex, endpoint, "GET", request.URL.String(), number, slot, &status, false, result.shouldRetry, "http_status", started)}
	result.execution.err = &Error{Kind: "HttpStatus", Service: OpenAlex, Endpoint: endpoint, StatusCode: status, Body: safeOpenAlexBody(endpoint, payload, query)}
	return result
}

func (live *LiveTransport) executeSemantic(ctx context.Context, url string, query []QueryPair, body any) (any, error) {
	deadline := live.logicalDeadline(ctx)
	maximum := max(len(live.config.SemanticScholarApiKeys), 1) + 2
	excluded := []int{}
	var lastError error
	for index := 0; index < maximum; index++ {
		reserved, err := reserveSemantic(ctx, live.semantic, excluded, deadline)
		if err != nil {
			if lastError != nil {
				return nil, lastError
			}
			return nil, err
		}
		result := live.semanticAttempt(ctx, reserved, deadline, url, query, body, index+1, maximum)
		live.recordExecutions(ctx, result.execution.attempts)
		if !result.shouldRetry {
			return result.execution.payload, result.execution.err
		}
		excluded = append(excluded, reserved.Slot)
		lastError = result.execution.err
	}
	return nil, lastError
}
func (live *LiveTransport) semanticAttempt(ctx context.Context, reserved reservation, deadline time.Time, url string, query []QueryPair, body any, number, maximum int) attemptResult {
	const endpoint = "paper_batch"
	result := attemptResult{}
	delay := seconds(1 << min(number-1, 5))
	slot := reserved.Slot
	if _, hasTime := transport.Remaining(deadline); !hasTime || ctx.Err() != nil {
		result.execution.err = sourceDeadlineError(SemanticScholar)
		return result
	}
	key := live.config.SemanticScholarApiKeys[slot]
	request, cancel, err := live.buildRequest(ctx, deadline, "POST", url, query, body, &key)
	if err != nil {
		live.semantic.finish(reserved, unixScheduleTime(), healthTerminal, scheduleTime{})
		result.execution.err = &Error{Kind: "Request", Service: SemanticScholar, Endpoint: endpoint, Message: "request build failed"}
		return result
	}
	defer cancel()
	started := time.Now()
	response, err := live.client.Transport.RoundTrip(request)
	if err != nil {
		result.shouldRetry = number < maximum
		live.semantic.finish(reserved, unixScheduleTime(), healthTransient, delay)
		result.execution.attempts = []executionAttempt{newAttempt(SemanticScholar, endpoint, "POST", request.URL.String(), number, slot, nil, false, result.shouldRetry, "transport", started)}
		result.execution.err = &Error{Kind: "Request", Service: SemanticScholar, Endpoint: endpoint, Message: "transport failure"}
		return result
	}
	status := uint16(response.StatusCode)
	retry := retryHeader(response.Header)
	isSuccess := status >= 200 && status < 300
	payload, bodyError := responseJson(response)
	if errors.Is(bodyError, transport.ErrInvalidJson) {
		if isSuccess {
			result.shouldRetry = number < maximum
			live.semantic.finish(reserved, unixScheduleTime(), healthTransient, delay)
			result.execution.attempts = []executionAttempt{newAttempt(SemanticScholar, endpoint, "POST", request.URL.String(), number, slot, &status, false, result.shouldRetry, "invalid_json", started)}
			result.execution.err = &Error{Kind: "Request", Service: SemanticScholar, Endpoint: endpoint, Message: "Semantic Scholar returned invalid JSON"}
			return result
		}
		payload = map[string]any{}
		bodyError = nil
	}
	if bodyError != nil {
		isTooLarge := errors.Is(bodyError, transport.ErrTooLarge)
		outcome := httpHealth(status)
		if isTooLarge {
			if status == 429 {
				outcome = healthRateLimited
			} else {
				outcome = healthTerminal
			}
		} else if isSuccess {
			outcome = healthTransient
		}
		result.shouldRetry = !isTooLarge && outcome != healthTerminal && number < maximum
		if retry != nil && (outcome == healthTransient || outcome == healthRateLimited) {
			delay = later(delay, *retry)
		}
		live.semantic.finish(reserved, unixScheduleTime(), outcome, delay)
		errorKind := "response_body"
		if isTooLarge {
			errorKind = "response_too_large"
		}
		result.execution.attempts = []executionAttempt{newAttempt(SemanticScholar, endpoint, "POST", request.URL.String(), number, slot, &status, false, result.shouldRetry, errorKind, started)}
		if isSuccess {
			result.execution.err = &Error{Kind: "Request", Service: SemanticScholar, Endpoint: endpoint, Message: bodyError.Error()}
		} else {
			result.execution.err = &Error{Kind: "HttpStatus", Service: SemanticScholar, Endpoint: endpoint, StatusCode: status, Body: safeSemanticBody(status, map[string]any{})}
		}
		return result
	}
	if isSuccess {
		live.semantic.finish(reserved, unixScheduleTime(), healthSuccess, scheduleTime{})
		result.execution.payload = payload
		result.execution.attempts = []executionAttempt{newAttempt(SemanticScholar, endpoint, "POST", request.URL.String(), number, slot, &status, true, false, "none", started)}
		return result
	}
	outcome := httpHealth(status)
	result.shouldRetry = outcome != healthTerminal && number < maximum
	if retry != nil && (outcome == healthTransient || outcome == healthRateLimited) {
		delay = later(delay, *retry)
	}
	live.semantic.finish(reserved, unixScheduleTime(), outcome, delay)
	result.execution.attempts = []executionAttempt{newAttempt(SemanticScholar, endpoint, "POST", request.URL.String(), number, slot, &status, false, result.shouldRetry, "http_status", started)}
	result.execution.err = &Error{Kind: "HttpStatus", Service: SemanticScholar, Endpoint: endpoint, StatusCode: status, Body: safeSemanticBody(status, payload)}
	return result
}
