package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/admission"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/danielgtaylor/huma/v2"
)

// requireProblem checks the standard error body and rejects the retired envelope fields.
func requireProblem(t testing.TB, response *httptest.ResponseRecorder, status int, detail string) huma.ErrorModel {
	t.Helper()
	var problem huma.ErrorModel
	if err := json.Unmarshal(response.Body.Bytes(), &problem); err != nil {
		t.Fatal(err, response.Body.String())
	}
	if response.Code != status || problem.Status != status || problem.Title != http.StatusText(status) || response.Header().Get("Content-Type") != "application/problem+json" {
		t.Fatal(response.Code, response.Header(), problem)
	}
	if detail != "" && problem.Detail != detail {
		t.Fatalf("detail %q; want %q", problem.Detail, detail)
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(response.Body.Bytes(), &fields); err != nil {
		t.Fatal(err)
	}
	if fields["code"] != nil || fields["retryable"] != nil {
		t.Fatal("retired envelope", response.Body.String())
	}
	return problem
}

// TestProblemResponsesRetainDetailsAndRetryHeaders checks business failures against Huma's model.
func TestProblemResponsesRetainDetailsAndRetryHeaders(t *testing.T) {
	for _, scenario := range []struct {
		failure *apiError
		retry   string
	}{
		{badRequest("bad filter"), ""},
		{serviceUnavailable(), "5"},
		{&apiError{status: 429, detail: "rate limited"}, "5"},
		{&apiError{status: 428, detail: "Sign in first", details: []error{&huma.ErrorDetail{Location: "code", Message: "login_required"}, &huma.ErrorDetail{Location: "phase", Message: "fulltext"}}}, ""},
		{&apiError{status: 408, detail: "timed out"}, ""},
	} {
		response := httptest.NewRecorder()
		scenario.failure.write(response)
		problem := requireProblem(t, response, scenario.failure.status, "")
		if response.Header().Get("Retry-After") != scenario.retry {
			t.Fatal(response.Header())
		}
		if len(scenario.failure.details) != 0 && (problem.Detail != "Sign in first" || len(problem.Errors) != 2 || problem.Errors[0].Location != "code" || problem.Errors[0].Message != "login_required") {
			t.Fatal(problem)
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
		{query.ErrLegacyWeeklyLimit, 413, "Weekly updates exceed 2000 articles; use /api/weekly-updates/summary and /api/weekly-updates/articles"},
		{errors.New("SQL error in private/path.sqlite, secret=fixture"), 500, "Internal Server Error"},
	} {
		failure := mapIndexError(scenario.failure)
		if failure.status != scenario.status || failure.detail != scenario.detail {
			t.Fatalf("mapped error: %+v", failure)
		}
	}
	if mapExecutorError(admission.ErrClosed).status != 503 || mapExecutorError(context.DeadlineExceeded).status != 503 || mapExecutorError(executor.ErrWorkerFailed).status != 500 || mapExecutorError(context.Canceled).status != 499 {
		t.Fatal("executor classification changed")
	}
}

func writeResponse(writer http.ResponseWriter, value any) {
	if err := httpwire.JSON(writer, 200, value); err != nil {
		internalError().write(writer)
	}
}
