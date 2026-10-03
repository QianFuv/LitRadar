package httpwire

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/google/jsonschema-go/jsonschema"
)

type fixtureArticle struct {
	ArticleId       identity.Id `json:"article_id"`
	JournalId       identity.Id `json:"journal_id"`
	IssueId         *int64      `json:"issue_id"`
	Title           string      `json:"title"`
	PublicationYear *int64      `json:"publication_year"`
	Date            *string     `json:"date"`
	DatePrecision   *string     `json:"date_precision"`
	Authors         []string    `json:"authors"`
	StartPage       *string     `json:"start_page"`
	EndPage         *string     `json:"end_page"`
	Abstract        *string     `json:"abstract"`
	Doi             *string     `json:"doi"`
	Pmid            *string     `json:"pmid"`
	InPress         *bool       `json:"in_press"`
	OpenAccess      *bool       `json:"open_access"`
	RetractionDois  []string    `json:"retraction_dois"`
	JournalTitle    string      `json:"journal_title"`
	Volume          *string     `json:"volume"`
	Number          *string     `json:"number"`
}

type streamingAsset struct {
	ctx       context.Context
	position  int64
	readCount *atomic.Int64
}

func (asset *streamingAsset) Seek(offset int64, whence int) (int64, error) {
	switch whence {
	case io.SeekStart:
		asset.position = offset
	case io.SeekCurrent:
		asset.position += offset
	case io.SeekEnd:
		asset.position = (64 << 20) + offset
	}
	return asset.position, nil
}

func (asset *streamingAsset) Read(buffer []byte) (int, error) {
	if asset.position >= 64<<10 {
		<-asset.ctx.Done()
		return 0, asset.ctx.Err()
	}
	count := min(len(buffer), (64<<10)-int(asset.position))
	for index := range buffer[:count] {
		buffer[index] = 'x'
	}
	asset.position += int64(count)
	asset.readCount.Add(int64(count))
	return count, nil
}

func TestFileStreamsBeforeEofAndCancelsReader(t *testing.T) {
	var count atomic.Int64
	finished := make(chan struct{})
	listener := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		defer close(finished)
		File(writer, request, "large.bin", "application/octet-stream", "no-store", time.Time{}, &streamingAsset{ctx: request.Context(), readCount: &count})
	}))
	defer listener.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	request, _ := http.NewRequestWithContext(ctx, "GET", listener.URL, nil)
	response, err := listener.Client().Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	first := make([]byte, 32768)
	if _, err := io.ReadFull(response.Body, first); err != nil {
		t.Fatal(err)
	}
	if response.ContentLength != 64<<20 || !bytes.Equal(first, bytes.Repeat([]byte{'x'}, 32768)) {
		t.Fatal("first chunk not streamed")
	}
	cancel()
	select {
	case <-finished:
	case <-time.After(2 * time.Second):
		t.Fatal("canceled file response retained handler")
	}
	if count.Load() > 64<<10 {
		t.Fatal("whole asset buffered")
	}
}

type fixturePage struct {
	Items []fixtureArticle `json:"items"`
	Page  struct {
		Total      *int64  `json:"total"`
		Limit      int64   `json:"limit"`
		Offset     int64   `json:"offset"`
		NextCursor *string `json:"next_cursor"`
		HasMore    *bool   `json:"has_more"`
	} `json:"page"`
}

func readFixture(t *testing.T, name string) []byte {
	t.Helper()
	body, err := os.ReadFile("../../../tests/data/" + name)
	if err != nil {
		t.Fatal(err)
	}
	return body
}

func decoded(t *testing.T, body []byte) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(body, &value); err != nil {
		t.Fatal(err)
	}
	return value
}

// TestRestListenerAgainstFrozenContract injects domain results at a private listener.
// It proves wire primitives, not implementation of the represented production routes.
func TestRestListenerAgainstFrozenContract(t *testing.T) {
	expected := readFixture(t, "scenarios/api/article-page.json")
	var page fixturePage
	if err := json.Unmarshal(expected, &page); err != nil {
		t.Fatal(err)
	}
	var openapi struct {
		Components struct {
			Schemas map[string]json.RawMessage `json:"schemas"`
		} `json:"components"`
	}
	if err := json.Unmarshal(readFixture(t, "migration/rust/openapi.json"), &openapi); err != nil {
		t.Fatal(err)
	}
	makeSchema := func(name string) *jsonschema.Resolved {
		t.Helper()
		body, err := json.Marshal(map[string]any{"$ref": "#/$defs/" + name, "$defs": openapi.Components.Schemas})
		if err != nil {
			t.Fatal(err)
		}
		body = bytes.ReplaceAll(body, []byte("#/components/schemas/"), []byte("#/$defs/"))
		var schema jsonschema.Schema
		if err := json.Unmarshal(body, &schema); err != nil {
			t.Fatal(err)
		}
		resolved, err := schema.Resolve(nil)
		if err != nil {
			t.Fatal(err)
		}
		return resolved
	}
	pageSchema, errorSchema := makeSchema("ArticlePage"), makeSchema("ErrorEnvelope")
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/articles", func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Authorization") != "Bearer synthetic-fixture" {
			_ = JSON(writer, 401, ErrorEnvelope{"Authentication required", "unauthorized", false})
			return
		}
		if _, err := strconv.ParseInt(request.URL.Query().Get("limit"), 10, 64); err != nil {
			_ = JSON(writer, 400, ErrorEnvelope{"Invalid integer for limit", "bad_request", false})
			return
		}
		_ = JSON(writer, 200, page)
	})
	mux.HandleFunc("GET /api/announcements", func(writer http.ResponseWriter, request *http.Request) { _ = JSON(writer, 200, []any{}) })
	mux.HandleFunc("GET /api/articles/9001/abstract", func(writer http.ResponseWriter, request *http.Request) {
		Redirect(writer, "https://doi.org/10.1234/fixture")
	})
	mux.HandleFunc("GET /_next/static/chunks/app-abc123.js", func(writer http.ResponseWriter, request *http.Request) {
		File(writer, request, "app-abc123.js", "text/javascript", "public, max-age=31536000, immutable", time.Time{}, strings.NewReader("console.log('asset');"))
	})
	server := httptest.NewServer(mux)
	defer server.Close()
	client := server.Client()
	client.Timeout = 5 * time.Second
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	for _, scenario := range []struct {
		name, path, method, authorization, rangeHeader string
		status                                         int
		body                                           string
		schema                                         *jsonschema.Resolved
	}{
		{"auth-before-query", "/api/articles?limit=abc", "GET", "", "", 401, `{"detail":"Authentication required","code":"unauthorized","retryable":false}`, errorSchema},
		{"query-error", "/api/articles?limit=abc", "GET", "Bearer synthetic-fixture", "", 400, `{"detail":"Invalid integer for limit","code":"bad_request","retryable":false}`, errorSchema},
		{"page", "/api/articles?db=scenario.sqlite&limit=10&offset=0&include_total=true", "GET", "Bearer synthetic-fixture", "", 200, string(expected), pageSchema},
		{"empty-array", "/api/announcements", "GET", "", "", 200, `[]`, nil},
		{"redirect", "/api/articles/9001/abstract?db=fixture", "GET", "", "", 307, "", nil},
		{"asset", "/_next/static/chunks/app-abc123.js", "GET", "", "", 200, "console.log('asset');", nil},
		{"range", "/_next/static/chunks/app-abc123.js", "GET", "", "bytes=0-6", 206, "console", nil},
		{"head", "/_next/static/chunks/app-abc123.js", "HEAD", "", "", 200, "", nil},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			request, _ := http.NewRequest(scenario.method, server.URL+scenario.path, nil)
			request.Header.Set("Authorization", scenario.authorization)
			if scenario.rangeHeader != "" {
				request.Header.Set("Range", scenario.rangeHeader)
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
			if response.StatusCode != scenario.status {
				t.Fatalf("status %d: %s", response.StatusCode, body)
			}
			if scenario.schema != nil {
				actual := decoded(t, body)
				if !reflect.DeepEqual(actual, decoded(t, []byte(scenario.body))) {
					t.Fatalf("contract mismatch: %s", body)
				}
				if err := scenario.schema.Validate(actual); err != nil {
					t.Fatal(err)
				}
				if response.Header.Get("Content-Type") != "application/json" {
					t.Fatal(response.Header)
				}
			} else if string(body) != scenario.body {
				t.Fatalf("body %q", body)
			}
			if scenario.status == 307 && (response.Header.Get("Location") != "https://doi.org/10.1234/fixture" || response.Header.Get("Cache-Control") != "private, no-store") {
				t.Fatal(response.Header)
			}
			if strings.HasPrefix(scenario.path, "/_next/") {
				if response.Header.Get("Content-Type") != "text/javascript" || response.Header.Get("Cache-Control") != "public, max-age=31536000, immutable" || response.Header.Get("Accept-Ranges") != "bytes" {
					t.Fatal(response.Header)
				}
				if scenario.status == 206 && response.Header.Get("Content-Range") != "bytes 0-6/21" {
					t.Fatal(response.Header)
				}
			}
		})
	}
	// A schema validator that accepts broken identifier types cannot serve as this gate.
	broken := decoded(t, expected).(map[string]any)
	broken["items"].([]any)[0].(map[string]any)["article_id"] = float64(9001)
	if pageSchema.Validate(broken) == nil {
		t.Fatal("numeric identifier negative control accepted")
	}
	page.Items[0].ArticleId = identity.Id(9223372036854775807)
	page.Items[0].Abstract = nil
	writer := httptest.NewRecorder()
	if err := JSON(writer, 200, page); err != nil {
		t.Fatal(err)
	}
	article := decoded(t, writer.Body.Bytes()).(map[string]any)["items"].([]any)[0].(map[string]any)
	if article["article_id"] != "9223372036854775807" {
		t.Fatal(article)
	}
	if value, exists := article["abstract"]; !exists || value != nil {
		t.Fatal("nullable property omitted")
	}
}
