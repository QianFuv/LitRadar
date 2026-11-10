package openapi

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/danielgtaylor/huma/v2"
)

// TestGenerateUsesGoContracts checks the generated document and the default error model.
func TestGenerateUsesGoContracts(t *testing.T) {
	bindings := []Operation{
		{"GET", "/health/live", "live"},
		{"POST", "/api/auth/login", "login"},
		{"GET", "/api/articles/{article_id}", "get_article"},
		{"POST", "/api/admin/scheduled-tasks", "create_scheduled_task"},
	}
	data, err := Generate(bindings)
	if err != nil {
		t.Fatal(err)
	}
	var document struct {
		Version string `json:"openapi"`
		Paths   map[string]map[string]struct {
			ID        string `json:"operationId"`
			Responses map[string]struct {
				Content map[string]any `json:"content"`
				Headers map[string]any `json:"headers"`
			} `json:"responses"`
		} `json:"paths"`
		Components struct {
			Schemas map[string]json.RawMessage `json:"schemas"`
		} `json:"components"`
	}
	if err := json.Unmarshal(data, &document); err != nil {
		t.Fatal(err)
	}
	if document.Version != "3.1.0" || len(document.Paths) != len(bindings) {
		t.Fatal(string(data))
	}
	for _, binding := range bindings {
		operation := document.Paths[binding.Path][strings.ToLower(binding.Method)]
		if operation.ID != binding.Id {
			t.Fatal(binding, operation.ID)
		}
		if operation.Responses["400"].Content["application/problem+json"] == nil || operation.Responses["503"].Headers["Retry-After"] == nil {
			t.Fatal("missing problem or retry contract", binding)
		}
	}
	for _, name := range []string{"LoginRequest", "LoginResponse", "ArticleRecord", "ErrorModel", "ScheduledTaskCreate", "ScheduledIndexSpec", "ScheduledNotifySpec", "ScheduledPushSpec"} {
		if document.Components.Schemas[name] == nil {
			t.Fatal("missing Go schema", name)
		}
	}
	if document.Components.Schemas["ErrorEnvelope"] != nil || strings.Contains(string(data), `"$schema"`) {
		t.Fatal("retired error or injected schema property")
	}
}

// TestGenerateRejectsInvalidBindings keeps registration failures explicit.
func TestGenerateRejectsInvalidBindings(t *testing.T) {
	for _, scenario := range []struct {
		bindings []Operation
		message  string
	}{
		{[]Operation{{"GET", "/health/live", "live"}, {"GET", "/health/live", "live"}}, "duplicate API binding: GET /health/live"},
		{[]Operation{{"GET", "/unknown", "unknown"}}, "unknown API operation: unknown"},
	} {
		if data, err := Generate(scenario.bindings); data != nil || err == nil || err.Error() != scenario.message {
			t.Fatal(string(data), err)
		}
	}
}

// TestHumaDocumentationUsesGeneratedSpec checks the built-in UI and its asset policy.
func TestHumaDocumentationUsesGeneratedSpec(t *testing.T) {
	mux := http.NewServeMux()
	New(mux)
	response := httptest.NewRecorder()
	mux.ServeHTTP(response, httptest.NewRequest("GET", "/docs", nil))
	if response.Code != 200 || !strings.HasPrefix(response.Header().Get("Content-Type"), "text/html") || !strings.Contains(response.Body.String(), "elements-api") || !strings.Contains(response.Body.String(), "/openapi.yaml") {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
	if !strings.Contains(response.Header().Get("Content-Security-Policy"), "https://unpkg.com/@stoplight/elements@") || !strings.Contains(response.Body.String(), "integrity=") {
		t.Fatal("documentation asset policy missing")
	}
	response = httptest.NewRecorder()
	mux.ServeHTTP(response, httptest.NewRequest("GET", "/docs/swagger-ui.js", nil))
	if response.Code != 404 {
		t.Fatal("retired Swagger asset remains", response.Code)
	}
}

// TestTypedValidationAndBodyReplay checks Huma admission before the business handler.
func TestTypedValidationAndBodyReplay(t *testing.T) {
	mux := http.NewServeMux()
	api := New(mux)
	calls := 0
	original := `{"username":"fixture","password":"password"}`
	err := Register(api, Operation{"POST", "/api/auth/login", "login"}, func(writer http.ResponseWriter, request *http.Request) {
		calls++
		data, err := io.ReadAll(request.Body)
		if err != nil || string(data) != original {
			t.Error("request body changed", string(data), err)
		}
		writer.Header().Set("Content-Type", "application/json")
		if _, err := writer.Write([]byte(`{"expires_at":1,"user":{"id":1}}`)); err != nil {
			t.Error(err)
		}
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		body, contentType string
		status            int
	}{
		{original, "application/json", 200},
		{`{}`, "application/json", 422},
		{`{"username":1,"password":"password"}`, "application/json", 422},
		{`{`, "application/json", 400},
		{original, "text/plain", 415},
		{strings.Repeat(" ", 2*1024*1024+1), "application/json", 413},
	} {
		request := httptest.NewRequest("POST", "/api/auth/login", strings.NewReader(scenario.body))
		request.Header.Set("Content-Type", scenario.contentType)
		response := httptest.NewRecorder()
		mux.ServeHTTP(response, request)
		if response.Code != scenario.status {
			t.Fatal(scenario, response.Code, response.Body.String())
		}
		if scenario.status >= 400 && response.Header().Get("Content-Type") != "application/problem+json" {
			t.Fatal(response.Header())
		}
		if response.Header().Get("Link") != "" || strings.Contains(response.Body.String(), `"$schema"`) {
			t.Fatal("schema injection enabled")
		}
	}
	if calls != 1 {
		t.Fatal("business handler admitted invalid requests", calls)
	}
}

// TestSchemaLinksAreDisabled checks normal Huma serialization as well as streamed operations.
func TestSchemaLinksAreDisabled(t *testing.T) {
	mux := http.NewServeMux()
	api := New(mux)
	type output struct{ Body HealthResponse }
	huma.Register(api, huma.Operation{Method: "GET", Path: "/probe"}, func(context.Context, *struct{}) (*output, error) {
		return &output{Body: HealthResponse{Status: "ok"}}, nil
	})
	response := httptest.NewRecorder()
	mux.ServeHTTP(response, httptest.NewRequest("GET", "/probe", nil))
	if response.Code != 200 || response.Header().Get("Link") != "" || strings.Contains(response.Body.String(), `"$schema"`) {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
}
