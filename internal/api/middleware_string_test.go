package api

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/openapi"
)

// stringResponseProbe records which write interface reaches the underlying response.
type stringResponseProbe struct {
	http.ResponseWriter
	stringWrites, byteWrites, headers int
	failure                           error
	shortCount                        int
}

// readerResponseProbe records the native bulk-copy path and its downstream result.
type readerResponseProbe struct {
	*stringResponseProbe
	readerCalls int
}

// ReadFrom exercises copying through byte writes or a configured short/error return.
func (writer *readerResponseProbe) ReadFrom(source io.Reader) (int64, error) {
	writer.readerCalls++
	if writer.shortCount > 0 {
		return int64(writer.shortCount), writer.failure
	}
	return io.Copy(writer.stringResponseProbe, source)
}

// Write records byte writes and can reproduce a short or failed downstream write.
func (writer *stringResponseProbe) Write(value []byte) (int, error) {
	writer.byteWrites++
	if writer.shortCount > 0 {
		return writer.shortCount, writer.failure
	}
	return writer.ResponseWriter.Write(value)
}

// WriteString records direct string writes with identical downstream failure behavior.
func (writer *stringResponseProbe) WriteString(value string) (int, error) {
	writer.stringWrites++
	if writer.shortCount > 0 {
		return writer.shortCount, writer.failure
	}
	return io.WriteString(writer.ResponseWriter, value)
}

// WriteHeader records when the outer policy commits response headers.
func (writer *stringResponseProbe) WriteHeader(status int) {
	writer.headers++
	writer.ResponseWriter.WriteHeader(status)
}

// TestPolicyStringWrites preserves mixed-write ordering and delayed HEAD response metadata.
func TestPolicyStringWrites(t *testing.T) {
	for _, isHead := range []bool{false, true} {
		t.Run(fmt.Sprintf("head=%t", isHead), func(t *testing.T) {
			recorder := httptest.NewRecorder()
			probe := &stringResponseProbe{ResponseWriter: recorder}
			prepared := 0
			writer := &policyResponse{ResponseWriter: probe, isHead: isHead, prepare: func(status int) {
				prepared++
				recorder.Header().Set("X-Prepared", strconv.Itoa(status))
			}}
			for _, value := range []string{"α", "tail"} {
				if count, err := io.WriteString(writer, value); err != nil || count != len(value) {
					t.Fatal(count, err)
				}
				writer.WriteHeader(202)
			}
			if count, err := writer.Write([]byte("!")); err != nil || count != 1 {
				t.Fatal(count, err)
			}
			if isHead && (probe.headers != 0 || prepared != 0 || probe.byteWrites != 0 || probe.stringWrites != 0) {
				t.Fatal("HEAD sent a body or prepared headers before finish", probe)
			}
			writer.Header().Set("X-Late", "retained")
			writer.finish()
			if recorder.Code != 200 || prepared != 1 || probe.headers != 1 || recorder.Header().Get("X-Prepared") != "200" {
				t.Fatal("implicit status or header preparation changed", recorder, probe)
			}
			if isHead {
				if recorder.Body.Len() != 0 || recorder.Result().Header.Get("Content-Length") != "7" || recorder.Result().Header.Get("X-Late") != "retained" {
					t.Fatal("HEAD lost byte length or late headers", recorder)
				}
			} else if recorder.Body.String() != "αtail!" || probe.stringWrites != 2 || probe.byteWrites != 1 {
				t.Fatal("string writes copied through the byte interface or reordered content", recorder.Body.String(), probe)
			}
		})
	}
}

// TestPolicyStringWriteFallbackAndErrors preserves byte-only writers and short-write return values.
func TestPolicyStringWriteFallbackAndErrors(t *testing.T) {
	for _, hasStrings := range []bool{false, true} {
		for _, failure := range []error{nil, io.ErrClosedPipe} {
			probe := &stringResponseProbe{ResponseWriter: httptest.NewRecorder(), shortCount: 2, failure: failure}
			var destination http.ResponseWriter = probe
			if !hasStrings {
				destination = struct{ http.ResponseWriter }{probe}
			}
			writer := &policyResponse{ResponseWriter: destination, prepare: func(int) {}}
			count, err := io.WriteString(writer, "payload")
			if count != 2 || !errors.Is(err, failure) || probe.headers != 1 || (probe.stringWrites == 1) != hasStrings {
				t.Fatal("downstream write result or interface changed", count, err, probe)
			}
		}
	}
}

// TestPolicyLargeStringCopies preserves bulk-copy byte counts and downstream failures.
func TestPolicyLargeStringCopies(t *testing.T) {
	value := strings.Repeat("x", 1<<20)
	for _, failure := range []error{nil, io.ErrClosedPipe} {
		probe := &readerResponseProbe{stringResponseProbe: &stringResponseProbe{ResponseWriter: httptest.NewRecorder()}}
		if failure != nil {
			probe.shortCount, probe.failure = 17, failure
		}
		writer := &policyResponse{ResponseWriter: probe, prepare: func(int) {}}
		count, err := io.WriteString(writer, value)
		expectedCount := len(value)
		if failure != nil {
			expectedCount = 17
		}
		if count != expectedCount || !errors.Is(err, failure) || probe.readerCalls != 1 || probe.headers != 1 || probe.stringWrites != 0 {
			t.Fatal("bulk-copy result or dispatch changed", count, err, probe)
		}
	}
}

// TestPolicyLargeStringHttpResponse preserves mixed bodies, declared lengths, and HEAD headers on a real socket.
func TestPolicyLargeStringHttpResponse(t *testing.T) {
	value := strings.Repeat("中<&>", 262144)
	expected := "prefix" + value + "suffix"
	for _, hasLength := range []bool{false, true} {
		handler := &Handler{options: Options{ContentSecurityPolicy: fixtureCsp}, routes: compileRoutes([]route{{openapi.Operation{Method: "GET", Path: "/api/large-string"}, func(writer http.ResponseWriter, request *http.Request) {
			writer.Header().Set("Content-Type", "text/plain; charset=utf-8")
			if hasLength {
				writer.Header().Set("Content-Length", strconv.Itoa(len(expected)))
			}
			_, _ = writer.Write([]byte("prefix"))
			if count, err := io.WriteString(writer, value); err != nil || count != len(value) {
				t.Error("large string write failed", count, err)
			}
			_, _ = io.WriteString(writer, "suffix")
		}}})}
		server := httptest.NewServer(handler)
		client := server.Client()
		client.Timeout = 5 * time.Second
		for _, method := range []string{"GET", "HEAD"} {
			request, _ := http.NewRequest(method, server.URL+"/api/large-string", nil)
			response, err := client.Do(request)
			if err != nil {
				server.Close()
				t.Fatal(err)
			}
			body, readError := io.ReadAll(response.Body)
			response.Body.Close()
			if readError != nil || response.StatusCode != 200 || response.Header.Get("Content-Security-Policy") != fixtureCsp ||
				method == "GET" && string(body) != expected || method == "HEAD" && (len(body) != 0 || response.ContentLength != int64(len(expected))) {
				server.Close()
				t.Fatal("HTTP string representation changed", method, len(body), response.Header, readError)
			}
		}
		server.Close()
	}
}

// TestPolicyOversizedDeclaredString preserves whole-call rejection even if headers are edited after commitment.
func TestPolicyOversizedDeclaredString(t *testing.T) {
	for _, shouldRemoveHeader := range []bool{false, true} {
		t.Run(fmt.Sprintf("remove-late=%t", shouldRemoveHeader), func(t *testing.T) {
			type writeResult struct {
				count int
				err   error
			}
			completed := make(chan writeResult, 1)
			server := httptest.NewServer(http.HandlerFunc(func(destination http.ResponseWriter, request *http.Request) {
				writer := &policyResponse{ResponseWriter: destination, prepare: func(int) {}}
				writer.Header().Set("Content-Length", "40000")
				writer.WriteHeader(http.StatusOK)
				if shouldRemoveHeader {
					writer.Header().Del("Content-Length")
				}
				count, err := io.WriteString(writer, strings.Repeat("x", 1<<20))
				completed <- writeResult{count, err}
				writer.finish()
			}))
			defer server.Close()
			client := server.Client()
			client.Timeout = 5 * time.Second
			response, err := client.Get(server.URL)
			if err != nil {
				t.Fatal(err)
			}
			body, _ := io.ReadAll(response.Body)
			response.Body.Close()
			result := <-completed
			if result.count != 0 || !errors.Is(result.err, http.ErrContentLength) || len(body) != 0 {
				t.Fatal("oversized declared write was partially published", result.count, result.err, len(body))
			}
		})
	}
}

// TestFavoriteExportStringResponse preserves authenticated attachment and HEAD behavior through the policy.
func TestFavoriteExportStringResponse(t *testing.T) {
	_, _, router, token := favoriteFixture(t)
	created := authRequest(router, "POST", "/api/favorites/folders", `{"name":"string-write"}`, token)
	var folder struct{ Id int64 }
	if created.Code != 200 || json.Unmarshal(created.Body.Bytes(), &folder) != nil {
		t.Fatal("folder creation failed", created)
	}
	path := fmt.Sprintf("/api/favorites/folders/%d", folder.Id)
	added := authRequest(router, "POST", path+"/articles", `{"article_id":1,"db_name":"unavailable"}`, token)
	if added.Code != 200 {
		t.Fatal("favorite creation failed", added)
	}
	var probe *stringResponseProbe
	wrapped := http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		probe = &stringResponseProbe{ResponseWriter: writer}
		response := &policyResponse{ResponseWriter: probe, isHead: request.Method == "HEAD", prepare: func(int) {}}
		router.ServeHTTP(response, request)
		response.finish()
	})
	get := authRequest(wrapped, "GET", path+"/export", "", token)
	if get.Code != 200 || get.Body.Len() == 0 || probe.stringWrites != 1 || probe.byteWrites != 0 {
		t.Fatal("export copied through byte writes", get, probe)
	}
	head := authRequest(wrapped, "HEAD", path+"/export", "", token)
	if head.Code != 200 || head.Body.Len() != 0 || head.Header().Get("Content-Length") != strconv.Itoa(get.Body.Len()) ||
		head.Header().Get("Content-Disposition") != get.Header().Get("Content-Disposition") || probe.stringWrites != 0 || probe.byteWrites != 0 {
		t.Fatal("HEAD attachment changed or wrote a body", head, probe)
	}
}

// BenchmarkPolicyHttpStringResponses includes the real HTTP middleware, socket, and client body drain.
func BenchmarkPolicyHttpStringResponses(b *testing.B) {
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(io.Discard, nil)))
	defer slog.SetDefault(previous)
	payload := map[string]any{"abstract": strings.Repeat("A", 256<<10)}
	encoded, err := jsonvalue.EncodeJson(payload)
	if err != nil {
		b.Fatal(err)
	}
	export := strings.Repeat("@article{fixture,title={Research}}\n", 32768)
	for _, format := range []string{"JSON", "Export"} {
		for _, method := range []string{"GET", "HEAD"} {
			for _, usesStrings := range []bool{false, true} {
				b.Run(fmt.Sprintf("%s/%s/strings=%t", format, method, usesStrings), func(b *testing.B) {
					expectedLength := len(export)
					if format == "JSON" {
						expectedLength = len(encoded)
					}
					handler := &Handler{options: Options{ContentSecurityPolicy: fixtureCsp}, routes: compileRoutes([]route{{openapi.Operation{Method: "GET", Path: "/api/benchmark"}, func(writer http.ResponseWriter, request *http.Request) {
						if !usesStrings {
							writer = struct{ http.ResponseWriter }{writer}
						}
						if format == "JSON" {
							writeResponse(writer, payload)
						} else {
							writer.Header().Set("Content-Type", "application/x-bibtex")
							_, _ = io.WriteString(writer, export)
						}
					}}})}
					server := httptest.NewServer(handler)
					defer server.Close()
					client := server.Client()
					client.Timeout = 5 * time.Second
					b.ReportAllocs()
					for b.Loop() {
						request, err := http.NewRequest(method, server.URL+"/api/benchmark", nil)
						if err != nil {
							b.Fatal(err)
						}
						response, err := client.Do(request)
						if err != nil {
							b.Fatal(err)
						}
						count, readError := io.Copy(io.Discard, response.Body)
						closeError := response.Body.Close()
						if readError != nil || closeError != nil || response.StatusCode != 200 {
							b.Fatal(count, readError, closeError, response.StatusCode)
						}
						if method == "GET" && count != int64(expectedLength) || method == "HEAD" && (count != 0 || response.ContentLength != int64(expectedLength)) {
							b.Fatal("response length changed", method, count, response.ContentLength)
						}
					}
				})
			}
		}
	}
}
