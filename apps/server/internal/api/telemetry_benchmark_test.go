package api

import (
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
)

// BenchmarkTelemetryRequest compares the complete middleware path with and without recording.
func BenchmarkTelemetryRequest(b *testing.B) {
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(io.Discard, nil)))
	defer slog.SetDefault(previous)
	for _, isEnabled := range []bool{false, true} {
		name := "disabled"
		handler := &Handler{mux: fixtureMux([]route{{openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}"}, func(writer http.ResponseWriter, request *http.Request) { writer.WriteHeader(200) }}})}
		if isEnabled {
			name = "enabled"
			handler.telemetry = telemetry.New()
		}
		request := httptest.NewRequest("GET", "/api/articles/1001", nil)
		b.Run(name, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				handler.ServeHTTP(httptest.NewRecorder(), request)
			}
		})
	}
}
