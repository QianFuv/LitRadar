package api

import (
	"log/slog"
	"net/http"
	"time"
)

// loadTelemetry requires administrator admission before reading bounded hourly data.
func (handlers *adminHandlers) loadTelemetry(writer http.ResponseWriter, request *http.Request) {
	if _, failure := handlers.auth.authenticator.requireAdmin(request); failure != nil {
		failure.write(writer)
		return
	}
	period := request.URL.Query().Get("period")
	hours := 24
	switch period {
	case "", "24h":
	case "7d":
		hours = 7 * 24
	case "30d":
		hours = 30 * 24
	default:
		badRequest("period must be 24h, 7d or 30d").write(writer)
		return
	}
	kind := request.URL.Query().Get("kind")
	switch kind {
	case "", "http", "mcp", "search":
	default:
		badRequest("kind must be http, mcp or search").write(writer)
		return
	}
	to := time.Now().UTC().Truncate(time.Hour).Add(time.Hour)
	report, err := handlers.telemetry.Read(request.Context(), to.Add(-time.Duration(hours)*time.Hour), to, kind)
	if err != nil {
		slog.WarnContext(request.Context(), "telemetry.read.failed", "component", "telemetry")
		(&apiError{status: http.StatusServiceUnavailable, detail: "Load telemetry temporarily unavailable"}).write(writer)
		return
	}
	writeResponse(writer, report)
}
