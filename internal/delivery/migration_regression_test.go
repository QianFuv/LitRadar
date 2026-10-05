package delivery

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/QianFuv/LitRadar/internal/testkit/testlog"
	"testing"
	"time"
)

func TestActualPushFailureLogsDoNotRetryOrDiscloseMessage(t *testing.T) {
	const sentinel = "private-push-sentinel"
	ctx, finish := testlog.Capture(t)
	fixture := &pushFixture{responses: []json.RawMessage{json.RawMessage(`{"status":503,"request_id":"request-456","retry_after":2,"body":{"error":"` + sentinel + `"}}`), json.RawMessage(`{"body":{"code":200,"data":"` + sentinel + `"}}`)}}
	client := NewPushplusClient(1, time.Second, nil)
	client.transport = fixture
	client.wait = func(context.Context, time.Duration) error { return nil }
	_, err := client.Send(ctx, PushplusMessage{Token: sentinel, Title: sentinel, Content: sentinel, Channel: "wechat", Template: "markdown"})
	var failure *PushplusError
	if !errors.As(err, &failure) || failure.Kind != "http_status" || failure.StatusCode != 503 {
		t.Fatal(err)
	}
	events := finish()
	failed := testlog.Events(events, "pushplus.request.failed")
	if len(failed) != 1 || len(fixture.requests) != 1 || len(fixture.responses) != 1 || len(testlog.Events(events, "pushplus.delivery.failed")) != 1 {
		t.Fatal(events)
	}
	testlog.Require(t, failed[0], map[string]any{"attempt": float64(1), "http_status": float64(503), "will_retry": false})
	span, ok := failed[0]["span"].(map[string]any)
	if !ok {
		t.Fatal(failed)
	}
	testlog.Require(t, span, map[string]any{"endpoint": "send"})
	testlog.Private(t, events, sentinel)
}
