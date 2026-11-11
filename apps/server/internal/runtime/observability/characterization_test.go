package observability

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"strings"
	"testing"
	"time"
)

func TestStandardHandlersKeepGroupsAndHideRoutingAttribute(t *testing.T) {
	for _, format := range []string{"json", "compact"} {
		t.Run(format, func(t *testing.T) {
			var output bytes.Buffer
			guard, logger, err := New("trace", format, &output)
			if err != nil {
				t.Fatal(err)
			}
			logger.WithGroup("request").Info("accepted", "id", 42, "log_target", "internal")
			guard.Shutdown()
			text := output.String()
			if strings.Contains(text, "log_target") {
				t.Fatal(text)
			}
			if format == "json" {
				var record struct {
					Time       time.Time
					Level, Msg string
					Request    struct {
						ID int `json:"id"`
					}
				}
				if json.Unmarshal(output.Bytes(), &record) != nil || record.Request.ID != 42 || record.Msg != "accepted" || record.Time.IsZero() {
					t.Fatal(text)
				}
			} else if !strings.Contains(text, "request.id=42") || !strings.Contains(text, "msg=accepted") {
				t.Fatal(text)
			}
		})
	}
}

func TestStandardJsonHandlerEncodesUnsupportedValuesWithoutLosingRecord(t *testing.T) {
	var output bytes.Buffer
	guard, logger, err := New("trace", "json", &output)
	if err != nil {
		t.Fatal(err)
	}
	record := slog.NewRecord(time.Unix(0, 0), slog.Level(-8), "event", 0)
	record.Add("invalid", make(chan int))
	if err := logger.Handler().Handle(context.Background(), record); err != nil {
		t.Fatal(err)
	}
	guard.Shutdown()
	var actual map[string]any
	if err := json.Unmarshal(output.Bytes(), &actual); err != nil {
		t.Fatal(err)
	}
	if actual["level"] != "TRACE" || !strings.HasPrefix(actual["invalid"].(string), "!ERROR:") {
		t.Fatal(actual)
	}
}
