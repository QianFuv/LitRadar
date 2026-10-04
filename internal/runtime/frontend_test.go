package runtime

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
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

func TestFrontendMatchesOriginalProductionWire(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/runtime/static-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		ExporterSha256 string `json:"exporter_sha256"`
		Modified       string
		Files          map[string]string
		GzipBase64     string `json:"gzip_base64"`
		Cases          []struct {
			Path, Method string
			Headers      map[string]string
			Response     struct {
				Status     int
				Headers    map[string]string
				BodySha256 string `json:"body_sha256"`
				BodyLength int    `json:"body_length"`
			}
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	exporter, err := os.ReadFile("../../tests/migration/runtime/export-static.mjs")
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.Sum256(exporter)
	if hex.EncodeToString(hash[:]) != corpus.ExporterSha256 {
		t.Fatal("original exporter identity changed")
	}
	configuration := runtimeConfiguration(t)
	root := filepath.Join(configuration.Storage.ProjectRoot, "web")
	modified, err := http.ParseTime(corpus.Modified)
	if err != nil {
		t.Fatal(err)
	}
	for name, content := range corpus.Files {
		filename := filepath.Join(root, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filename, []byte(content), 0600); err != nil {
			t.Fatal(err)
		}
		if err := os.Chtimes(filename, modified, modified); err != nil {
			t.Fatal(err)
		}
	}
	compressed, err := base64.StdEncoding.DecodeString(corpus.GzipBase64)
	if err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(root, "asset.js.gz")
	if err := os.WriteFile(filename, compressed, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(filename, modified, modified); err != nil {
		t.Fatal(err)
	}
	manifest, err := buildCspManifest(root)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "csp-hashes.json"), encoded, 0600); err != nil {
		t.Fatal(err)
	}
	configuration.IsDevelopment = false
	prepared, err := prepareResources(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	defer prepared.Close()
	server := httptest.NewServer(prepared.handler)
	defer server.Close()
	transport := &http.Transport{DisableCompression: true}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	for index, test := range corpus.Cases {
		t.Run(strconv.Itoa(index)+test.Method+test.Path, func(t *testing.T) {
			request, err := http.NewRequest(test.Method, server.URL+test.Path, nil)
			if err != nil {
				t.Fatal(err)
			}
			for name, value := range test.Headers {
				request.Header.Set(name, value)
			}
			response, err := client.Do(request)
			if err != nil {
				t.Fatal(err)
			}
			defer response.Body.Close()
			body, err := io.ReadAll(response.Body)
			if err != nil {
				t.Fatal(err)
			}
			if response.StatusCode != test.Response.Status {
				t.Errorf("status %d != %d", response.StatusCode, test.Response.Status)
			}
			for _, name := range []string{"content-type", "content-length", "content-encoding", "content-range", "accept-ranges", "last-modified", "etag", "vary", "allow", "location", "cache-control"} {
				actual := strings.Join(response.Header.Values(name), ", ")
				if actual != test.Response.Headers[name] {
					t.Errorf("%s: %q != %q", name, actual, test.Response.Headers[name])
				}
			}
			hash := sha256.Sum256(body)
			if len(body) != test.Response.BodyLength || hex.EncodeToString(hash[:]) != test.Response.BodySha256 {
				t.Errorf("body differs: %d bytes %q", len(body), body)
			}
		})
	}
}
