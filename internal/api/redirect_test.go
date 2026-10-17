package api

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestFrontendRedirectStaysOnOrigin preserves local navigation even for network-path input.
func TestFrontendRedirectStaysOnOrigin(t *testing.T) {
	handler := &Handler{}
	for _, method := range []string{http.MethodGet, http.MethodHead} {
		for _, scenario := range []struct{ path, location string }{
			{"/page/", "/page"},
			{"/page///?query=value", "/page?query=value"},
			{"//example.invalid/", "/example.invalid"},
			{"///example.invalid/path///?query=value", "/example.invalid/path?query=value"},
			{"//example.invalid/%2Fpath/?", "/example.invalid/%2Fpath?"},
		} {
			t.Run(method+scenario.path, func(t *testing.T) {
				request := httptest.NewRequest(method, "https://litradar.test"+scenario.path, nil)
				response := httptest.NewRecorder()
				handler.ServeHTTP(response, request)
				if response.Code != http.StatusPermanentRedirect || response.Header().Get("Location") != scenario.location {
					t.Fatalf("status=%d location=%q", response.Code, response.Header().Get("Location"))
				}
				destination, err := request.URL.Parse(response.Header().Get("Location"))
				if err != nil || destination.Host != request.URL.Host || destination.Scheme != request.URL.Scheme {
					t.Fatalf("redirect escaped origin: destination=%v err=%v", destination, err)
				}
			})
		}
	}
}
