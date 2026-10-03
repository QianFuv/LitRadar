package scholarly

import (
	"context"
	"errors"
	"log/slog"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func crossrefAttemptLimit(timeout uint64) int {
	selected := 3
	for attempts := 4; attempts <= 6; attempts++ {
		envelope := saturatingMultiply(max(timeout, 1), uint64(attempts))
		for index := 1; index < attempts; index++ {
			envelope = saturatingAdd(envelope, 1<<uint(index-1))
		}
		if envelope > 180 {
			break
		}
		selected = attempts
	}
	return selected
}
func (live *LiveTransport) executeCrossref(ctx context.Context, url string, query []QueryPair) (any, error) {
	const endpoint = "journal_works"
	deadline := live.logicalDeadline(ctx)
	maximum := crossrefAttemptLimit(live.config.TimeoutSeconds)
	for number := 1; number <= maximum; number++ {
		if !live.crossref.wait(ctx, deadline) {
			return nil, sourceDeadlineError(Crossref)
		}
		if _, hasTime := transport.Remaining(deadline); !hasTime || ctx.Err() != nil {
			return nil, sourceDeadlineError(Crossref)
		}
		request, cancel, err := live.buildRequest(ctx, deadline, "GET", url, query, nil, nil)
		if err != nil {
			return nil, &Error{Kind: "Request", Service: Crossref, Endpoint: endpoint, Message: "request build failed"}
		}
		started := time.Now()
		response, err := live.client.Transport.RoundTrip(request)
		if err != nil {
			cancel()
			delay := seconds(1 << uint(number-1))
			shouldRetry := number < maximum && transport.CanWait(delay.timer(), deadline)
			record := newAttempt(Crossref, endpoint, "GET", url, number, 0, nil, false, shouldRetry, "transport", started)
			message := "transport failure"
			record.attempt.Error = &message
			live.recordCrossref(ctx, record)
			if shouldRetry {
				if !transport.RetrySleep(ctx, delay.timer(), deadline) {
					return nil, sourceDeadlineError(Crossref)
				}
				continue
			}
			return nil, &Error{Kind: "Request", Service: Crossref, Endpoint: endpoint, Message: message}
		}
		status := uint16(response.StatusCode)
		retry := retryHeader(response.Header)
		outcome := httpHealth(status)
		isRetryable := outcome == healthRateLimited || outcome == healthTransient
		if isRetryable && retry != nil {
			live.crossref.deferFor(*retry)
		}
		delay := seconds(uint64(number))
		if retry != nil {
			delay = later(delay, *retry)
		}
		payload, bodyError := responseJson(response)
		cancel()
		isTooLarge := false
		if errors.Is(bodyError, transport.ErrInvalidJson) {
			payload = map[string]any{"error": transport.ErrInvalidJson.Error()}
			bodyError = nil
		}
		if bodyError != nil {
			if status == 429 {
				isTooLarge = errors.Is(bodyError, transport.ErrTooLarge)
				payload = map[string]any{"error": "source rate limited"}
			} else {
				errorKind := "response_body"
				if errors.Is(bodyError, transport.ErrTooLarge) {
					errorKind = "response_too_large"
				}
				record := newAttempt(Crossref, endpoint, "GET", request.URL.String(), number, 0, &status, false, false, errorKind, started)
				message := bodyError.Error()
				record.attempt.Error = &message
				live.recordCrossref(ctx, record)
				return nil, &Error{Kind: "Request", Service: Crossref, Endpoint: endpoint, Message: message}
			}
		}
		if status < 200 || status >= 300 {
			shouldRetry := isRetryable && number < 3 && !isTooLarge && transport.CanWait(delay.timer(), deadline)
			record := newAttempt(Crossref, endpoint, "GET", request.URL.String(), number, 0, &status, false, shouldRetry, "http_status", started)
			record.attempt.Error = optionalString(field(payload, "error"))
			live.recordCrossref(ctx, record)
			if shouldRetry {
				if !transport.RetrySleep(ctx, delay.timer(), deadline) {
					return nil, sourceDeadlineError(Crossref)
				}
				continue
			}
			if isRetryable {
				live.crossref.deferFor(delay)
			}
			return nil, &Error{Kind: "HttpStatus", Service: Crossref, Endpoint: endpoint, StatusCode: status, Body: payload}
		}
		live.recordCrossref(ctx, newAttempt(Crossref, endpoint, "GET", request.URL.String(), number, 0, &status, true, false, "none", started))
		return payload, nil
	}
	return nil, &Error{Kind: "Request", Service: Crossref, Endpoint: endpoint, Message: "request retry loop exhausted"}
}
func (live *LiveTransport) recordCrossref(ctx context.Context, record executionAttempt) {
	if !record.attempt.DidSucceed {
		status := uint16(0)
		if record.attempt.StatusCode != nil {
			status = *record.attempt.StatusCode
		}
		slog.WarnContext(ctx, "source request failed", "event", "source.request.failed", "component", "source", "provider", Crossref, "endpoint", record.attempt.Endpoint, "method", record.attempt.Method, "attempt", record.Number, "outcome", "failure", "error_kind", record.ErrorKind, "http_status", status, "has_http_status", record.attempt.StatusCode != nil, "is_retry", record.attempt.DidRetry, "will_retry", record.WillRetry, "duration_ms", record.DurationMillis)
	}
	live.attempts = append(live.attempts, record.attempt)
}
