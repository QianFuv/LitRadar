package delivery

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
)

type pushFixture struct {
	responses []json.RawMessage
	requests  []any
	after     func()
}

func (fixture *pushFixture) PostJson(ctx context.Context, location string, headers http.Header, body any, timeout time.Duration) (outbound.Response, error) {
	if _, err := sources.Json(body); err != nil {
		return outbound.Response{}, err
	}
	fixture.requests = append(fixture.requests, map[string]any{"url": location, "body": body})
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

func TestPushplusDoesNotConsumeAiBudgetAndCancellationStopsRetry(t *testing.T) {
	isCancelled := false
	control := domain.NewExecutionControl(float64(time.Now().Unix()+60), 0, func() (bool, error) { return isCancelled, nil })
	fixture := &pushFixture{responses: []json.RawMessage{json.RawMessage(`{"body":{"code":200}}`)}}
	client := NewPushplusClient(10, time.Second, control)
	client.transport = fixture
	if _, err := client.Send(context.Background(), PushplusMessage{}); err != nil {
		t.Fatalf("non-AI request consumed AI budget: %v", err)
	}
	fixture.responses = []json.RawMessage{json.RawMessage(`{"error":"connect_failed"}`)}
	fixture.after = func() { isCancelled = true }
	_, err := client.Send(context.Background(), PushplusMessage{})
	if !errors.Is(err, domain.ControlCancelled) || len(fixture.requests) != 2 {
		t.Fatalf("cancelled retry sent: %v count=%d", err, len(fixture.requests))
	}
}
