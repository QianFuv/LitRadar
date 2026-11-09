package openapi

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// setDocumentFixtures restores embedded declarations after a package-local characterization.
func setDocumentFixtures(t *testing.T, metadata, declared string) {
	t.Helper()
	originalDefinitions, originalOperations := definitions, operations
	t.Cleanup(func() { definitions, operations = originalDefinitions, originalOperations })
	definitions, operations = []byte(metadata), []byte(declared)
}

// TestGeneratePreservesBindingIdentityAndDocumentEncoding checks the observable schema format.
func TestGeneratePreservesBindingIdentityAndDocumentEncoding(t *testing.T) {
	setDocumentFixtures(t, `{"openapi":"3.0.3","note":"<>&","paths":{"ignored":{}}}`, `[{"method":"GET","path":"/one","operation":{"operationId":"read"}},{"method":"POST","path":"/one","operation":{"operationId":"write"}}]`)
	output, err := Generate([]Operation{{"POST", "/one", "write"}, {"GET", "/one", "read"}})
	expected := "{\n  \"note\": \"<>&\",\n  \"openapi\": \"3.0.3\",\n  \"paths\": {\n    \"/one\": {\n      \"get\": {\n        \"operationId\": \"read\"\n      },\n      \"post\": {\n        \"operationId\": \"write\"\n      }\n    }\n  }\n}\n"
	if err != nil || string(output) != expected {
		t.Fatal(string(output), err)
	}
}

// TestGenerateRetainsValidationPrecedence checks duplicate, missing and identity errors.
func TestGenerateRetainsValidationPrecedence(t *testing.T) {
	for _, scenario := range []struct {
		name, metadata, declared, message string
		bindings                          []Operation
	}{
		{"metadata first", `{`, `[`, "unexpected end of JSON input", nil},
		{"declarations before duplicates", `{}`, `[`, "unexpected end of JSON input", []Operation{{"GET", "/one", "read"}, {"GET", "/one", "read"}}},
		{"duplicate", `{}`, `[]`, "duplicate API binding: GET /one", []Operation{{"GET", "/one", "read"}, {"GET", "/one", "read"}}},
		{"missing before malformed identity", `{}`, `[{"method":"GET","path":"/one","operation":2}]`, "missing API binding: GET /one", nil},
		{"malformed identity", `{}`, `[{"method":"GET","path":"/one","operation":2}]`, "json: cannot unmarshal number into Go value of type struct { OperationId string \"json:\\\"operationId\\\"\" }", []Operation{{"GET", "/one", "read"}}},
		{"mismatch", `{}`, `[{"method":"GET","path":"/one","operation":{"operationId":"other"}}]`, "API binding identity mismatch: GET /one", []Operation{{"GET", "/one", "read"}}},
		{"remaining", `{}`, `[]`, "undocumented API bindings: 2", []Operation{{"GET", "/one", "read"}, {"GET", "/two", "read"}}},
		{"duplicate declaration consumed", `{}`, `[{"method":"GET","path":"/one","operation":{"operationId":"read"}},{"method":"GET","path":"/one","operation":{"operationId":"read"}}]`, "missing API binding: GET /one", []Operation{{"GET", "/one", "read"}}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			setDocumentFixtures(t, scenario.metadata, scenario.declared)
			output, err := Generate(scenario.bindings)
			if output != nil || err == nil || err.Error() != scenario.message {
				t.Fatal(string(output), err)
			}
		})
	}
}

// TestServeDocsPreservesPathAdmission checks exact redirects and resource rejection.
func TestServeDocsPreservesPathAdmission(t *testing.T) {
	for _, scenario := range []struct {
		urlPath, contentType, body string
		status                     int
	}{
		{"/docs", "", "", http.StatusSeeOther},
		{"/docs/" + string([]byte{255}), "text/plain; charset=utf-8", "Invalid URL: Invalid UTF-8 in `rest`", http.StatusBadRequest},
		{"/docs/nested/index.html", "", "", http.StatusNotFound},
		{"/docs/LICENSE", "", "", http.StatusNotFound},
		{"/docs/NOTICE", "", "", http.StatusNotFound},
		{"/docs/README.md", "", "", http.StatusNotFound},
		{"/docs/missing.js", "", "", http.StatusNotFound},
	} {
		request := httptest.NewRequest(http.MethodGet, "/docs/", nil)
		request.URL.Path = scenario.urlPath
		writer := httptest.NewRecorder()
		ServeDocs(writer, request)
		if writer.Code != scenario.status || writer.Header().Get("Content-Type") != scenario.contentType || writer.Body.String() != scenario.body {
			t.Fatal(scenario.urlPath, writer.Code, writer.Header(), writer.Body.String())
		}
		if scenario.status == http.StatusSeeOther && writer.Header().Get("Location") != "/docs/" {
			t.Fatal(writer.Header())
		}
	}
}

// TestServeDocsPreservesAssetBodies checks root aliases, MIME types and initializer injection.
func TestServeDocsPreservesAssetBodies(t *testing.T) {
	for _, scenario := range []struct{ name, asset, contentType string }{
		{"", "index.html", "text/html"},
		{"/", "index.html", "text/html"},
		{"index.css", "index.css", "text/css"},
		{"swagger-ui.js.map", "swagger-ui.js.map", "text/plain"},
		{"favicon-16x16.png", "favicon-16x16.png", "image/png"},
		{"swagger-initializer.js", "swagger-initializer.js", "text/javascript"},
	} {
		expected, err := swaggerAssets.ReadFile("swagger/" + scenario.asset)
		if err != nil {
			t.Fatal(err)
		}
		if scenario.asset == "swagger-initializer.js" {
			expected = []byte(strings.ReplaceAll(string(expected), "{{config}}", "  \"dom_id\": \"#swagger-ui\",\n  \"url\": \"/openapi.json\",\n  \"deepLinking\": true,\n  \"layout\": \"StandaloneLayout\""))
		}
		writer := httptest.NewRecorder()
		ServeDocs(writer, httptest.NewRequest(http.MethodGet, "/docs/"+scenario.name, nil))
		if writer.Code != http.StatusOK || writer.Header().Get("Content-Type") != scenario.contentType || !bytes.Equal(writer.Body.Bytes(), expected) {
			t.Fatal(scenario.name, writer.Code, writer.Header())
		}
	}
}
