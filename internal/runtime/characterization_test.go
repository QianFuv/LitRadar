package runtime

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

// TestRuntimeSettingsRetainEarlierPolicyOnLaterKnownValidationFailure checks sequential partial updates.
func TestRuntimeSettingsRetainEarlierPolicyOnLaterKnownValidationFailure(t *testing.T) {
	configuration, err := NewConfig(t.TempDir(), "127.0.0.1", 0, "key")
	if err != nil {
		t.Fatal(err)
	}
	configuration.AreSecureCookiesRequired = true
	err = configuration.ApplyRuntimeSettings([]settings.Value{{Field: "cors_allowed_origins", Value: "https://example.test"}, {Field: "log_format", Value: "invalid"}, {Field: "secure_cookies", Value: "yes"}})
	if err == nil || !reflect.DeepEqual(configuration.ApiOptions.CorsOrigins, []string{"https://example.test"}) || configuration.ApiOptions.AreCookiesSecure {
		t.Fatalf("sequential policy changed: %+v %v", configuration.ApiOptions, err)
	}
}

// frontendResponseFixture provides static content for precise range and fallback response assertions.
func frontendResponseFixture(t *testing.T) frontend {
	t.Helper()
	root := t.TempDir()
	for name, body := range map[string]string{"index.html": "home", "empty.txt": "", "file.txt": "abcdef", "404.html": "missing"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
	}
	return frontend{root}
}

// TestFrontendResponsePreservesFallbackHeadAndRangeOrder checks status and headers at decision boundaries.
func TestFrontendResponsePreservesFallbackHeadAndRangeOrder(t *testing.T) {
	files := frontendResponseFixture(t)
	for _, item := range []struct {
		method, path, rangeValue, body, length, contentRange string
		status                                               int
	}{
		{"GET", "/empty.txt", "bytes=0-", "", "0", "bytes 0-0/0", 206},
		{"GET", "/file.txt", "bytes=0-1", "ab", "2", "bytes 0-1/6", 206},
		{"GET", "/file.txt", "bytes=0-1,3-4", "Cannot serve multipart range requests", "37", "bytes */6", 416},
		{"HEAD", "/file.txt", "bytes=9-", "", "", "bytes */6", 416},
		{"GET", "/missing.txt", "bytes=0-1", "mi", "2", "bytes 0-1/7", 404},
		{"HEAD", "/%00.html", "", "", "", "", 500},
		{"GET", "/%00.html", "", "missing", "7", "", 404},
		{"POST", "/file.txt", "", "", "0", "", 405},
	} {
		request := httptest.NewRequest(item.method, item.path, nil)
		if item.rangeValue != "" {
			request.Header.Set("Range", item.rangeValue)
		}
		response := httptest.NewRecorder()
		files.ServeHTTP(response, request)
		assertFrontendResponse(t, response, item.status, item.body, item.length, item.contentRange)
	}
}

// assertFrontendResponse checks exact HTTP metadata as well as response body.
func assertFrontendResponse(t *testing.T, response *httptest.ResponseRecorder, status int, body, length, contentRange string) {
	t.Helper()
	if response.Code != status || response.Body.String() != body || response.Header().Get("Content-Length") != length || response.Header().Get("Content-Range") != contentRange {
		t.Fatalf("response: %d %+v %q", response.Code, response.Header(), response.Body.String())
	}
	if status == http.StatusMethodNotAllowed && response.Header().Get("Allow") != "GET,HEAD" {
		t.Fatal("method admission changed")
	}
}
