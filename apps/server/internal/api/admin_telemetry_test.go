package api

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"net/url"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/content"
)

// TestAdminTelemetryAuthorizationAndShape checks role admission, filters and pending data.
func TestAdminTelemetryAuthorizationAndShape(t *testing.T) {
	_, router, admin, _, member := adminFixture(t)
	for _, scenario := range []struct {
		token  string
		status int
	}{{"", 401}, {member, 403}, {admin, 200}} {
		response := authRequest(router, "GET", "/api/admin/telemetry", "", scenario.token)
		if response.Code != scenario.status {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	auth, _, token := authFixture(t)
	collector := telemetry.New()
	collector.Mcp("search_articles", false, time.Millisecond)
	collector.Http("/api/articles/{article_id}", "GET", 404, 2*time.Millisecond)
	handlers := adminHandlers{auth: auth, telemetry: collector}
	for _, scenario := range []struct {
		query          string
		status, series int
	}{
		{"", 200, 2}, {"?period=7d&kind=mcp", 200, 1}, {"?period=30d&kind=search", 200, 0},
		{"?period=1d", 422, 0}, {"?kind=private", 422, 0},
	} {
		response := authRequest(httpTelemetryHandler(&handlers), "GET", "/api/admin/telemetry"+scenario.query, "", token)
		if response.Code != scenario.status {
			t.Fatal(scenario, response.Code, response.Body.String())
		}
		if scenario.status != 200 {
			continue
		}
		var report telemetry.Report
		if err := json.Unmarshal(response.Body.Bytes(), &report); err != nil || !report.Enabled || len(report.Series) != scenario.series || report.Hourly == nil || report.SearchShapes == nil {
			t.Fatal(report, err)
		}
		if len(report.Series) > 0 && (report.Series[0].Calls != 1 || report.Series[0].P99Ms <= 0) {
			t.Fatal(report.Series)
		}
	}
}

// httpTelemetryHandler exposes the concrete administrator handler for focused admission checks.
func httpTelemetryHandler(handlers *adminHandlers) *Handler {
	return &Handler{mux: fixtureMux(handlers.routes()[:1])}
}

// TestHttpSearchTelemetryPrivacyStoredRows exercises real auth, routes and MATCH execution before inspecting SQLite.
func TestHttpSearchTelemetryPrivacyStoredRows(t *testing.T) {
	auth, _, token := authFixture(t)
	configuration := config.FromProjectRoot(t.TempDir())
	if err := content.Create(filepath.Join(configuration.IndexDir, "catalog.sqlite"), 9); err != nil {
		t.Fatal(err)
	}
	collector, err := telemetry.Open(context.Background(), configuration.TelemetryDbPath(), 90)
	if err != nil {
		t.Fatal(err)
	}
	defer collector.Close()
	indexes := indexHandlers{storage: configuration, authenticator: auth.authenticator, pool: auth.pool}
	handler := &Handler{mux: fixtureMux(indexes.routes()), telemetry: collector}
	secret := "private_search_canary"
	for _, path := range []string{"/api/articles?q=" + url.QueryEscape(secret), "/api/articles/private_identifier_canary", "/api/private_path_canary?token=private_query_canary"} {
		request := httptest.NewRequest("GET", path, nil)
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("X-Forwarded-For", "192.0.2.123")
		request.Header.Set("Cookie", "private_cookie_canary")
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		if response.Code >= 500 {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	if err := collector.Flush(context.Background(), time.Now()); err != nil {
		t.Fatal(err)
	}
	database, err := sqlite.OpenPlain(configuration.TelemetryDbPath())
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	rows, err := database.Query("SELECT dimensions,aggregate FROM hourly_load")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	count, hasSearch, hasTemplate := 0, false, false
	for rows.Next() {
		var labels, aggregate string
		if err := rows.Scan(&labels, &aggregate); err != nil {
			t.Fatal(err)
		}
		for _, forbidden := range []string{secret, token, "api_fixture", "192.0.2.123", "private_identifier_canary", "private_path_canary", "private_query_canary", "private_cookie_canary", "user_id", "username", "authorization"} {
			if strings.Contains(labels+aggregate, forbidden) {
				t.Fatal("privacy leak", forbidden, labels)
			}
		}
		hasSearch = hasSearch || strings.Contains(labels, `"kind":"search"`)
		hasTemplate = hasTemplate || strings.Contains(labels, "/api/articles/{article_id}")
		count++
	}
	if err := rows.Err(); err != nil || count != 4 || !hasSearch || !hasTemplate {
		t.Fatal(count, hasSearch, hasTemplate, err)
	}
}

// TestAdminTelemetryUnavailableIsRetryable keeps store-read failures on the retryable 503 contract.
func TestAdminTelemetryUnavailableIsRetryable(t *testing.T) {
	auth, _, token := authFixture(t)
	filename := filepath.Join(t.TempDir(), "telemetry.sqlite")
	collector, err := telemetry.Open(context.Background(), filename, 90)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(collector.Close)
	database, err := sqlite.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("DROP TABLE hourly_load"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	handlers := adminHandlers{auth: auth, telemetry: collector}
	response := authRequest(httpTelemetryHandler(&handlers), "GET", "/api/admin/telemetry", "", token)
	if response.Code != 503 || response.Header().Get("Retry-After") == "" || !strings.Contains(response.Body.String(), "Load telemetry temporarily unavailable") {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
}
