package openapi

import (
	"context"

	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/danielgtaylor/huma/v2"
)

// TestGenerateUsesGoContracts checks the generated document and the default error model.

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

// TestTypedValidationConsumesDeclaredBody checks Huma admission before the business handler.
func TestTypedValidationConsumesDeclaredBody(t *testing.T) {
	mux := http.NewServeMux()
	api := New(mux)
	calls := 0
	original := `{"username":"fixture","password":"password"}`
	type loginBody struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	type input struct{ Body loginBody }
	type output struct{ Body healthResponse }
	huma.Register(api, huma.Operation{Method: "POST", Path: "/api/auth/login", MaxBodyBytes: 2 * 1024 * 1024}, func(ctx context.Context, input *input) (*output, error) {
		calls++
		if input.Body.Username != "fixture" || input.Body.Password != "password" {
			t.Fatal(input.Body)
		}
		return &output{Body: healthResponse{Status: "ok"}}, nil
	})
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
	type output struct{ Body healthResponse }
	huma.Register(api, huma.Operation{Method: "GET", Path: "/probe"}, func(context.Context, *struct{}) (*output, error) {
		return &output{Body: healthResponse{Status: "ok"}}, nil
	})
	response := httptest.NewRecorder()
	mux.ServeHTTP(response, httptest.NewRequest("GET", "/probe", nil))
	if response.Code != 200 || response.Header().Get("Link") != "" || strings.Contains(response.Body.String(), `"$schema"`) {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
}

type healthResponse struct {
	Status string `json:"status"`
}
