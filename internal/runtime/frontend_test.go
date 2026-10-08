package runtime

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"io/fs"
	"net/http"
	"net/http/httptest"

	"os"
	"path/filepath"

	"testing"
	"testing/fstest"
)

func runtimeConfiguration(t *testing.T) Config {
	t.Helper()
	root := t.TempDir()
	key := filepath.Join(root, "secret.key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	configuration, err := NewConfig(root, "127.0.0.1", 0, key)
	if err != nil {
		t.Fatal(err)
	}
	configuration.IsDevelopment = true
	return configuration
}

// TestEmbeddedFrontendPreservesHttpContract checks immutable assets without disk-backed file metadata.
func TestEmbeddedFrontendPreservesHttpContract(t *testing.T) {
	source := fstest.MapFS{
		"index.html":            {Data: []byte("home")},
		"404.html":              {Data: []byte("missing")},
		"page.html":             {Data: []byte("page")},
		"_next/static/chunk.js": {Data: []byte("abcdef")},
	}
	var compressed bytes.Buffer
	writer := gzip.NewWriter(&compressed)
	writer.Write(source["_next/static/chunk.js"].Data)
	writer.Close()
	source["_next/static/chunk.js.gz"] = &fstest.MapFile{Data: compressed.Bytes()}
	files, err := newEmbeddedFrontend(source)
	if err != nil {
		t.Fatal(err)
	}
	for _, item := range []struct {
		method, path, encoding, rangeValue, body string
		status                                   int
	}{
		{"GET", "/", "", "", "home", 200},
		{"GET", "/page", "", "", "page", 200},
		{"GET", "/_next/static/chunk.js", "", "bytes=1-3", "bcd", 206},
		{"HEAD", "/_next/static/chunk.js", "", "", "", 200},
		{"GET", "/_next/static/chunk.js", "gzip", "", compressed.String(), 200},
		{"GET", "/_next/static/chunk.js", "gzip;q=0,identity;q=1", "", "abcdef", 200},
		{"GET", "/missing", "", "", "missing", 404},
		{"GET", "/%2e%2e/secret", "", "", "missing", 404},
		{"POST", "/", "", "", "", 405},
	} {
		request := httptest.NewRequest(item.method, item.path, nil)
		request.Header.Set("Accept-Encoding", item.encoding)
		if item.rangeValue != "" {
			request.Header.Set("Range", item.rangeValue)
		}
		response := httptest.NewRecorder()
		files.ServeHTTP(response, request)
		if response.Code != item.status || response.Body.String() != item.body || response.Header().Get("Last-Modified") != "" {
			t.Fatalf("%s %s: %d %q %+v", item.method, item.path, response.Code, response.Body.String(), response.Header())
		}
	}
	identity := files.validators["_next/static/chunk.js"]
	if identity == "" || identity == files.validators["_next/static/chunk.js.gz"] {
		t.Fatal("representation validators collide")
	}
	request := httptest.NewRequest("GET", "/_next/static/chunk.js", nil)
	request.Header.Set("If-None-Match", identity)
	response := httptest.NewRecorder()
	files.ServeHTTP(response, request)
	if response.Code != http.StatusNotModified {
		t.Fatal(response.Code)
	}
	request.Header.Del("If-None-Match")
	request.Header.Set("If-Modified-Since", "Wed, 01 Jan 2031 00:00:00 GMT")
	response = httptest.NewRecorder()
	files.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatal("unknown modification time admitted a date validator", response.Code)
	}
	source["_next/static/chunk.js"].Data = []byte("uvwxyz")
	updated, err := newEmbeddedFrontend(source)
	if err != nil || updated.validators["_next/static/chunk.js"] == identity {
		t.Fatal("same-size revisions collide", err)
	}
}

// TestEmbeddedCspRejectsMissingAndChangedBytes proves validation uses the served filesystem.
func TestEmbeddedCspRejectsMissingAndChangedBytes(t *testing.T) {
	source := fstest.MapFS{"index.html": {Data: []byte("<script>ready=true;</script>")}}
	if _, err := loadSecurityPolicyFiles(source); err == nil {
		t.Fatal("missing manifest admitted")
	}
	manifest, err := buildCspManifestFiles(source)
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	source["csp-hashes.json"] = &fstest.MapFile{Data: data}
	if _, err := loadSecurityPolicyFiles(source); err != nil {
		t.Fatal(err)
	}
	source["index.html"].Data = []byte("<script>ready=false;</script>")
	if _, err := loadSecurityPolicyFiles(source); err == nil {
		t.Fatal("stale manifest admitted changed HTML")
	}
	source["index.html"].Mode = fs.ModeSymlink
	if _, err := loadSecurityPolicyFiles(source); err == nil {
		t.Fatal("linked HTML admitted")
	}
}
