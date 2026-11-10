package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
	"log/slog"
	"time"
)

// loadTelemetry requires administrator admission before reading bounded hourly data.
func (handlers *adminHandlers) loadTelemetry(ctx context.Context, input *TelemetryInput) (*output[telemetry.Report], error) {
	request := input.request
	if _, failure := handlers.auth.authenticator.requireAdmin(request); failure != nil {
		return nil, failure.problem()
	}
	period := input.Period
	hours := 24
	switch period {
	case "", "24h":
	case "7d":
		hours = 7 * 24
	case "30d":
		hours = 30 * 24
	default:
		return nil, badRequest("period must be 24h, 7d or 30d").problem()
	}
	kind := textOr(input.Kind.Pointer(), "")
	switch kind {
	case "", "http", "mcp", "search":
	default:
		return nil, badRequest("kind must be http, mcp or search").problem()
	}
	to := time.Now().UTC().Truncate(time.Hour).Add(time.Hour)
	report, err := handlers.telemetry.Read(request.Context(), to.Add(-time.Duration(hours)*time.Hour), to, kind)
	if err != nil {
		slog.WarnContext(request.Context(), "telemetry.read.failed", "component", "telemetry")
		retry := uint64(5)
		return nil, (&apiError{status: 503, detail: "Load telemetry temporarily unavailable", retryAfter: &retry}).problem()
	}
	return &output[telemetry.Report]{Body: report}, nil
}
