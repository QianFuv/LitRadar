package jfbym

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"

	"reflect"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestLiveRequestAndFailureBoundaries(t *testing.T) {
	for _, test := range []struct {
		name   string
		status int
		body   string
		want   string
	}{
		{"success", 200, `{"code":10000,"data":{"data":"261.5"}}`, ""},
		{"redirect", 302, strings.Repeat("x", maximumResponse+1), "jfbym HTTP status 302"},
		{"http-error", 429, `not-json`, "jfbym HTTP status 429"},
		{"too-large", 200, strings.Repeat("x", maximumResponse+1), "jfbym response exceeded the configured size limit"},
		{"invalid-json", 200, `not-json`, "jfbym response is not valid JSON"},
		{"failed-recognition", 200, `{"code":10001,"msg":"secret-token slide-image background-image"}`, "jfbym recognition failed with code Some(10001)"},
		{"float-code", 200, `{"code":10000.0,"data":{"data":261}}`, "jfbym recognition failed with code None"},
		{"unsigned-code", 200, `{"code":18446744073709551615}`, "jfbym recognition failed with code Some(-1)"},
		{"missing-distance", 200, `{"code":10000,"data":{}}`, "jfbym response missing slider distance"},
	} {
		t.Run(test.name, func(t *testing.T) {
			var count atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				count.Add(1)
				if request.Method != "POST" || request.Header.Get("Content-Type") != "application/json" {
					t.Error("request contract", request.Method, request.Header)
				}
				body, err := io.ReadAll(request.Body)
				if err != nil {
					t.Error(err)
				}
				var payload map[string]string
				json.Unmarshal(body, &payload)
				if !reflect.DeepEqual(payload, map[string]string{"token": "secret-token", "type": "20111", "slide_image": "slide-image", "background_image": "background-image"}) {
					t.Error("request body mismatch")
				}
				writer.Header().Set("Location", "http://127.0.0.1:9/must-not-follow")
				writer.WriteHeader(test.status)
				io.WriteString(writer, test.body)
			}))
			defer server.Close()
			solver, err := NewLive("secret-token", 2, transport.Proxy{}, time.Time{})
			if err != nil {
				t.Fatal(err)
			}
			defer solver.Close()
			solver.apiUrl = server.URL
			value, err := solver.SolveDualImage(context.Background(), "data:image/png;base64,slide-image", "background-image")
			if test.want == "" {
				if err != nil || value != 261.5 {
					t.Fatal(value, err)
				}
			} else if err == nil || err.Error() != test.want {
				t.Fatalf("%v want %s", err, test.want)
			}
			if count.Load() != 1 {
				t.Fatal("unexpected retry or redirect", count.Load())
			}
			for _, secret := range []string{"secret-token", "slide-image", "background-image"} {
				if strings.Contains(fmt.Sprintf("%v %#v %v", solver, solver, err), secret) {
					t.Fatal("secret leaked")
				}
			}
		})
	}
}

func TestDeadlineAndFixturePrecedence(t *testing.T) {
	if _, err := NewLive(" ", 1, transport.Proxy{}, time.Time{}); err == nil || err.Error() != "jfbym token is required" {
		t.Fatal(err)
	}
	solver, err := NewLive("secret", 0, transport.Proxy{}, time.Now().Add(-time.Second))
	if err != nil {
		t.Fatal(err)
	}
	defer solver.Close()
	if solver.requestTimeout != time.Second {
		t.Fatal("minimum timeout changed")
	}
	if _, err := solver.SolveDualImage(context.Background(), "", ""); err == nil || err.Error() != "article access deadline expired" {
		t.Fatal(err)
	}
	fixture := NewFixture(120, 1)
	if _, err := fixture.SolveDualImage(context.Background(), "", "bg"); err == nil {
		t.Fatal("empty image accepted")
	}
	if _, err := fixture.SolveDualImage(context.Background(), "slide", "bg"); err == nil || err.Error() != "jfbym fixture forced failure" {
		t.Fatal(err)
	}
	if value, err := fixture.SolveDualImage(context.Background(), "slide", "bg"); err != nil || value != 120 {
		t.Fatal(value, err)
	}
}

func TestSolverFormattingRedactsPointersAndValues(t *testing.T) {
	solver, err := NewLive("private-token-sentinel", 1, transport.Proxy{}, time.Time{})
	if err != nil {
		t.Fatal(err)
	}
	defer solver.Close()
	for _, value := range []any{solver, *solver} {
		var output strings.Builder
		logger := slog.New(slog.NewJSONHandler(&output, nil))
		logger.Info("solver", slog.Any("value", value))
		output.WriteString(fmt.Sprintf("%v %+v %#v", value, value, value))
		if strings.Contains(output.String(), "private-token-sentinel") {
			t.Fatal("solver token leaked")
		}
	}
}

func TestLiveBodyCancellationAndTruncation(t *testing.T) {
	for _, isTruncated := range []bool{false, true} {
		t.Run(strconv.FormatBool(isTruncated), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				writer.Header().Set("Content-Length", "100")
				io.WriteString(writer, `{"code":`)
				writer.(http.Flusher).Flush()
				if !isTruncated {
					<-request.Context().Done()
				}
			}))
			defer server.Close()
			solver, err := NewLive("secret", 1, transport.Proxy{}, time.Now().Add(50*time.Millisecond))
			if err != nil {
				t.Fatal(err)
			}
			defer solver.Close()
			solver.apiUrl = server.URL
			_, err = solver.SolveDualImage(context.Background(), "slide", "background")
			if err == nil || err.Error() != "jfbym response body could not be read" {
				t.Fatal(err)
			}
		})
	}
}
