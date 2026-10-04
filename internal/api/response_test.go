package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http/httptest"
	"testing"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

func TestErrorEnvelopesRetainStructuredDetailsAndRetrySemantics(t *testing.T) {
	for _, scenario := range []struct {
		failure *apiError
		retry   string
		fields  int
		code    string
	}{
		{badRequest("bad filter"), "", 3, "bad_request"},
		{serviceUnavailable(), "5", 3, "service_unavailable"},
		{&apiError{status: 428, structured: map[string]any{"code": "login_required"}}, "", 1, ""},
		{&apiError{status: 408, detail: "timed out"}, "", 3, "request_failed"},
	} {
		recorder := httptest.NewRecorder()
		scenario.failure.write(recorder)
		var payload map[string]any
		if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
			t.Fatal(err)
		}
		if recorder.Code != scenario.failure.status || recorder.Header().Get("Content-Type") != "application/json" || recorder.Header().Get("Retry-After") != scenario.retry || len(payload) != scenario.fields {
			t.Fatalf("response: %d %v %v", recorder.Code, recorder.Header(), payload)
		}
		if scenario.fields == 3 && (payload["code"] != scenario.code || payload["retryable"] != (scenario.retry != "")) {
			t.Fatalf("envelope: %v", payload)
		}
	}
}

func TestStorageAndWorkerFailuresDoNotExposePrivateDetails(t *testing.T) {
	for _, scenario := range []struct {
		failure error
		status  int
		detail  string
	}{
		{config.ErrNoDatabases, 404, "No SQLite databases found"},
		{query.NotFound{Message: "Article not found"}, 404, "Article not found"},
		{query.InvalidInput{Message: "bad filter"}, 400, "bad filter"},
		{query.ErrLegacyWeeklyLimit, 413, query.ErrLegacyWeeklyLimit.Error()},
		{errors.New("SQL error in private/path.sqlite, secret=fixture"), 500, "Internal Server Error"},
	} {
		failure := mapIndexError(scenario.failure)
		if failure.status != scenario.status || failure.detail != scenario.detail {
			t.Fatalf("mapped error: %+v", failure)
		}
	}
	if mapExecutorError(admission.ErrClosed).status != 503 || mapExecutorError(context.DeadlineExceeded).status != 503 || mapExecutorError(executor.ErrWorkerFailed).status != 500 {
		t.Fatal("executor classification changed")
	}
}
