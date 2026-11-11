package api

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/danielgtaylor/huma/v2"
)

// TestCanceledRelatedRequestIsClientClosed checks cancellation during admitted work and its terminal log.
func TestCanceledRelatedRequestIsClientClosed(t *testing.T) {
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	pool := executor.New(1, time.Second)
	defer pool.Wait()
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	handler := &Handler{mux: fixtureMux([]route{fixtureRawRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}/related"}, func(writer http.ResponseWriter, request *http.Request) {
		_, err := admitted(request.Context(), pool, func() (string, error) {
			cancel()
			return "", errors.New("private SQL interrupted after client cancellation")
		}, mapIndexError)
		if failure, ok := errors.AsType[huma.StatusError](err); ok {
			writer.WriteHeader(failure.GetStatus())
		} else {
			t.Errorf("expected canceled operation, got %v", err)
		}
	})})}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequestWithContext(ctx, "GET", "/api/articles/9223361829786370983/related?private=secret", nil))
	pool.Wait()
	if response.Code != statusClientClosedRequest {
		t.Fatal(response.Code)
	}
	var record struct {
		Level, Outcome string
		Status         int
		ErrorKind      string `json:"error_kind"`
	}
	if err := json.Unmarshal(logs.Bytes(), &record); err != nil {
		t.Fatal(err, logs.String())
	}
	if record.Level != "INFO" || record.Outcome != "client_closed" || record.Status != 499 || record.ErrorKind != "context_canceled" {
		t.Fatal(record)
	}
	if strings.Contains(logs.String(), "private") || strings.Contains(logs.String(), "secret") || strings.Contains(logs.String(), "9223361829786370983") {
		t.Fatal("cancellation log disclosed request or error data", logs.String())
	}
}

// TestRequestFailureLogsSafeSqliteDiagnostics retains a wrapped SQLite error code without SQL or paths.
func TestRequestFailureLogsSafeSqliteDiagnostics(t *testing.T) {
	database, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Error(err)
		}
	}()
	_, cause := database.ExecContext(t.Context(), "SELECT private_secret FROM private_table")
	if cause == nil {
		t.Fatal("expected a SQLite error")
	}
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	request := withRequestID(httptest.NewRequest("GET", "/", nil), "fixture-request")
	failure := requestFailure(request.Context(), fmt.Errorf("private/path.sqlite: %w", cause), mapIndexError)
	if failure.status != 500 || failure.detail != "Internal Server Error" {
		t.Fatal(failure)
	}
	var record struct {
		Event      string
		ErrorKind  string `json:"error_kind"`
		ErrorType  string `json:"error_type"`
		SQLiteCode int    `json:"sqlite_code"`
		RequestID  string `json:"request_id"`
	}
	if err := json.Unmarshal(logs.Bytes(), &record); err != nil {
		t.Fatal(err, logs.String())
	}
	if record.Event != "http.operation.failed" || record.ErrorKind != "sqlite_error" || record.ErrorType != "*sqlite.Error" || record.SQLiteCode != 1 || record.RequestID != "fixture-request" {
		t.Fatal(record)
	}
	if strings.Contains(logs.String(), "private") {
		t.Fatal("internal error log disclosed SQL or paths", logs.String())
	}
}
