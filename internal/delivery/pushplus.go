// Package delivery orchestrates durable recommendation and notification work.
package delivery

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
)

const PushplusEndpoint = "https://www.pushplus.plus/send"

// PushplusMessage holds one notification and its optional routing fields.
type PushplusMessage struct {
	Token, Title, Content, Channel, Template string
	Topic, Option, To                        *string
}

func (message PushplusMessage) String() string       { return "PushplusMessage([REDACTED])" }
func (message PushplusMessage) GoString() string     { return message.String() }
func (message PushplusMessage) LogValue() slog.Value { return slog.StringValue(message.String()) }

// PushplusError preserves the delivery outcome classification without upstream response content.
type PushplusError struct {
	Kind              string
	StatusCode        int
	RequestId         *string
	RetryAfterSeconds *uint64
	Code              *int64
	message           string
	control           error
}

func (err *PushplusError) Error() string {
	switch err.Kind {
	case "connect_failed":
		return "PushPlus connection failed"
	case "timeout":
		return "PushPlus request timed out"
	case "http_status":
		message := fmt.Sprintf("PushPlus request failed with HTTP %d", err.StatusCode)
		if err.RequestId != nil {
			message += " (request ID: " + *err.RequestId + ")"
		}
		return message
	case "api_error":
		if err.Code == nil {
			return "PushPlus failed with code None"
		}
		return fmt.Sprintf("PushPlus failed with code Some(%d)", *err.Code)
	default:
		return err.message
	}
}

// Unwrap retains durable control errors for workflow finalization.
func (err *PushplusError) Unwrap() error { return err.control }

// PushplusClient retries only connection establishment failures to avoid duplicate sends.
type PushplusClient struct {
	transport     outbound.JsonTransport
	retryAttempts int
	timeout       time.Duration
	control       *domain.ExecutionControl
	wait          func(context.Context, time.Duration) error
}

// NewPushplusClient creates a fixed-endpoint, bounded HTTPS notification client.
func NewPushplusClient(retryAttempts int, timeout time.Duration, control *domain.ExecutionControl) *PushplusClient {
	timeout = max(time.Second, timeout)
	return &PushplusClient{outbound.New(timeout), min(max(retryAttempts, 0), 10), timeout, control, outbound.Wait}
}

// Send preserves success message IDs and stops after any potentially delivered attempt.
func (client *PushplusClient) Send(ctx context.Context, message PushplusMessage) (messageId string, resultError error) {
	ctx = observability.StartSpan(ctx, "litradar_worker::pushplus", "pushplus.delivery", map[string]any{"component": "delivery", "provider": "pushplus", "endpoint": "send"})
	started := time.Now()
	logger := slog.Default().With("component", "delivery", "provider", "pushplus", "endpoint", "send")
	logger.InfoContext(ctx, "pushplus.delivery.started", "event", "pushplus.delivery.started", "outcome", "started")
	defer func() {
		if resultError == nil {
			logger.InfoContext(ctx, "pushplus.delivery.completed", "event", "pushplus.delivery.completed", "outcome", "success", "duration_ms", time.Since(started).Milliseconds())
			return
		}
		var failure *PushplusError
		errors.As(resultError, &failure)
		logger.WarnContext(ctx, "pushplus.delivery.failed", "event", "pushplus.delivery.failed", "outcome", "failure", "error_kind", failure.Kind, "duration_ms", time.Since(started).Milliseconds())
	}()
	return client.sendAttempts(ctx, logger, message)
}

func pushplusBody(message PushplusMessage) any {
	body := map[string]any{"token": message.Token, "title": message.Title, "content": message.Content, "channel": message.Channel, "template": message.Template}
	for name, value := range map[string]*string{"topic": message.Topic, "option": message.Option, "to": message.To} {
		if value != nil && strings.TrimSpace(*value) != "" {
			body[name] = *value
		}
	}
	return body
}

func pushplusResponse(response outbound.Response) (string, *PushplusError) {
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return "", &PushplusError{Kind: "http_status", StatusCode: response.StatusCode, RequestId: response.RequestId, RetryAfterSeconds: response.RetryAfterSeconds}
	}
	object, ok := response.Body.(map[string]any)
	if !ok {
		return "", &PushplusError{Kind: "invalid_response", message: "PushPlus response is not a JSON object"}
	}
	code := pushplusResponseCode(object["code"])
	if code == nil || *code != 200 {
		return "", &PushplusError{Kind: "api_error", Code: code}
	}
	data := object["data"]
	if data == nil {
		return "", nil
	}
	if text, ok := data.(string); ok {
		return text, nil
	}
	encoded, err := sources.Json(data)
	if err != nil {
		return "", &PushplusError{Kind: "invalid_response", message: "PushPlus response is not a JSON object"}
	}
	return string(encoded), nil
}

func controlPushplusError(err error) *PushplusError {
	var control domain.ControlError
	if !errors.As(err, &control) {
		control = domain.ControlCancelled
	}
	return &PushplusError{Kind: string(control), message: control.Error(), control: control}
}

func (client *PushplusClient) pushplusAttemptTimeout() (time.Duration, error) {
	timeout := client.timeout
	if client.control != nil {
		var err error
		timeout, err = client.control.BeginExternalRequest(timeout)
		if err != nil {
			return 0, err
		}
	}
	return timeout, nil
}

func (client *PushplusClient) sendPushplusAttempt(ctx context.Context, message PushplusMessage, timeout time.Duration) (string, outbound.Response, *PushplusError) {
	response, err := client.transport.PostJson(ctx, PushplusEndpoint, nil, pushplusBody(message), timeout)
	var failure *PushplusError
	var messageId string
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
		failure = &PushplusError{Kind: kind, message: message}
	} else {
		messageId, failure = pushplusResponse(response)
	}
	return messageId, response, failure
}

func logPushplusAttemptFailure(ctx context.Context, logger *slog.Logger, failure *PushplusError, attempt int, willRetry bool, attemptStarted time.Time) {
	attributes := []any{"event", "pushplus.request.failed", "outcome", "failure", "attempt", attempt + 1, "error_kind", failure.Kind, "will_retry", willRetry, "duration_ms", time.Since(attemptStarted).Milliseconds()}
	if failure.Kind == "http_status" {
		attributes = append(attributes, "http_status", failure.StatusCode)
	}
	logger.WarnContext(ctx, "pushplus.request.failed", attributes...)
}

func (client *PushplusClient) waitPushplusRetry(ctx context.Context, delay time.Duration) error {
	if client.control != nil {
		return client.control.Wait(ctx, delay)
	} else {
		return client.wait(ctx, delay)
	}
}

func (client *PushplusClient) sendAttempts(ctx context.Context, logger *slog.Logger, message PushplusMessage) (string, error) {
	for attempt := 0; attempt <= client.retryAttempts; attempt++ {
		timeout, err := client.pushplusAttemptTimeout()
		if err != nil {
			return "", controlPushplusError(err)
		}
		attemptStarted := time.Now()
		messageId, response, failure := client.sendPushplusAttempt(ctx, message, timeout)
		if failure == nil {
			logger.InfoContext(ctx, "pushplus.request.completed", "event", "pushplus.request.completed", "outcome", "success", "attempt", attempt+1, "http_status", response.StatusCode, "duration_ms", time.Since(attemptStarted).Milliseconds())
			return messageId, nil
		}
		willRetry := attempt < client.retryAttempts && failure.Kind == "connect_failed"
		logPushplusAttemptFailure(ctx, logger, failure, attempt, willRetry, attemptStarted)
		if !willRetry {
			return "", failure
		}
		delay := outbound.RetryDelay(attempt, nil)
		err = client.waitPushplusRetry(ctx, delay)
		if err != nil {
			return "", controlPushplusError(err)
		}
	}
	return "", &PushplusError{Kind: "invalid_response", message: "PushPlus request was not attempted"}
}

func pushplusResponseCode(raw any) *int64 {
	var code *int64
	switch value := raw.(type) {
	case string:
		if number, err := strconv.ParseInt(value, 10, 64); err == nil {
			code = &number
		}
	case json.Number:
		if parsed, err := sources.ParseNumber(value); err == nil {
			if number, ok := parsed.AsInt64(); ok {
				code = &number
			}
		}
	case sources.Number:
		if number, ok := value.AsInt64(); ok {
			code = &number
		}
	case int64:
		code = &value
	}
	return code
}
