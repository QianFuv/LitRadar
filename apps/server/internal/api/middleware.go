package api

import (
	"crypto/rand"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/runtime/observability"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
)

// ServeHTTP applies the public security, correlation, CORS and cache contract.
func (handler *Handler) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	started := time.Now()
	request = request.WithContext(telemetry.WithCollector(request.Context(), handler.telemetry))
	var identifier [16]byte
	_, _ = rand.Read(identifier[:])
	identifier[6] = identifier[6]&0x0f | 0x40
	identifier[8] = identifier[8]&0x3f | 0x80
	id := fmt.Sprintf("%x-%x-%x-%x-%x", identifier[:4], identifier[4:6], identifier[6:8], identifier[8:10], identifier[10:])
	request = withRequestId(request, id)
	selected, label, allow := handler.route(request)
	request = request.WithContext(observability.StartSpan(request.Context(), "litradar_api::http_observability", "http.request", map[string]any{"component": "http", "request_id": id, "method": methodLabel(request.Method), "route": label}))
	response := &policyResponse{ResponseWriter: writer, isHead: request.Method == "HEAD"}
	isPreflight := request.Method == "OPTIONS"
	response.hasMatchedHead = selected != nil || allow != ""
	response.prepare = func(status int) {
		headers := writer.Header()
		headers.Set("X-Request-Id", id)
		if request.URL.Path != "/docs" || headers.Get("Content-Security-Policy") == "" {
			headers.Set("Content-Security-Policy", handler.options.ContentSecurityPolicy)
		}
		headers.Set("X-Content-Type-Options", "nosniff")
		headers.Set("Referrer-Policy", "same-origin")
		headers.Set("Permissions-Policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()")
		headers.Set("X-Frame-Options", "DENY")
		if handler.options.IsHstsEnabled {
			headers.Set("Strict-Transport-Security", "max-age=31536000")
		}
		headers.Set("Access-Control-Allow-Credentials", "true")
		headers.Add("Vary", "origin, access-control-request-method, access-control-request-headers")
		if origin := request.Header.Get("Origin"); origin != "" && slices.Contains(handler.options.CorsOrigins, origin) {
			headers.Set("Access-Control-Allow-Origin", origin)
		}
		if isPreflight {
			for _, pair := range [][2]string{{"Access-Control-Request-Method", "Access-Control-Allow-Methods"}, {"Access-Control-Request-Headers", "Access-Control-Allow-Headers"}} {
				if values, exists := request.Header[pair[0]]; exists && len(values) > 0 {
					headers.Set(pair[1], values[0])
				}
			}
		} else {
			headers.Set("Access-Control-Expose-Headers", "x-request-id,retry-after")
			applyCachePolicy(headers, request, status)
		}
	}
	switch {
	case isPreflight:
		if allow != "" {
			response.Header().Set("Allow", allow)
		}
		response.WriteHeader(http.StatusOK)
	case selected != nil:
		selected.ServeHTTP(response, request)
	case allow != "":
		response.Header().Set("Allow", allow)
		(&apiError{status: http.StatusMethodNotAllowed, detail: http.StatusText(http.StatusMethodNotAllowed)}).write(response)
	default:
		(&apiError{status: http.StatusNotFound, detail: http.StatusText(http.StatusNotFound)}).write(response)
	}
	response.finish()
	duration := time.Since(started)
	handler.telemetry.Http(label, methodLabel(request.Method), response.status, duration)
	logRequest(request, id, label, response.status, duration)
}

type policyResponse struct {
	http.ResponseWriter
	prepare           func(int)
	status            int
	isHead            bool
	hasMatchedHead    bool
	hasDeclaredLength bool
	length            int64
}

func (writer *policyResponse) Unwrap() http.ResponseWriter { return writer.ResponseWriter }
func (writer *policyResponse) WriteHeader(status int) {
	if writer.status != 0 {
		return
	}
	writer.status = status
	if !writer.isHead {
		writer.prepare(status)
		writer.hasDeclaredLength = writer.Header().Get("Content-Length") != ""
		writer.ResponseWriter.WriteHeader(status)
	}
}
func (writer *policyResponse) Write(data []byte) (int, error) {
	if writer.status == 0 {
		writer.WriteHeader(http.StatusOK)
	}
	if writer.isHead {
		writer.length += int64(len(data))
		return len(data), nil
	}
	return writer.ResponseWriter.Write(data)
}

// WriteString avoids HEAD body copies and uses bounded server copy buffers for large responses.
func (writer *policyResponse) WriteString(data string) (int, error) {
	if writer.status == 0 {
		writer.WriteHeader(http.StatusOK)
	}
	if writer.isHead {
		writer.length += int64(len(data))
		return len(data), nil
	}
	if destination, ok := writer.ResponseWriter.(io.ReaderFrom); ok {
		if len(data) >= 1<<20 && !writer.hasDeclaredLength && writer.Header().Get("Content-Length") == "" {
			count, err := destination.ReadFrom(struct{ io.Reader }{strings.NewReader(data)})
			return int(count), err
		}
		return writer.ResponseWriter.Write([]byte(data))
	}
	return io.WriteString(writer.ResponseWriter, data)
}

func (writer *policyResponse) Flush() { _ = writer.FlushError() }
func (writer *policyResponse) FlushError() error {
	if writer.status == 0 {
		writer.WriteHeader(http.StatusOK)
	}
	if writer.isHead {
		return nil
	}
	return http.NewResponseController(writer.ResponseWriter).Flush()
}
func (writer *policyResponse) finish() {
	if writer.status == 0 {
		writer.WriteHeader(http.StatusOK)
	}
	if writer.isHead {
		if writer.Header().Get("Content-Length") == "" && (writer.length > 0 || writer.hasMatchedHead) && writer.status >= 200 && writer.status != 204 && writer.status != 304 {
			writer.Header().Set("Content-Length", strconv.FormatInt(writer.length, 10))
		}
		writer.prepare(writer.status)
		writer.ResponseWriter.WriteHeader(writer.status)
	}
}

// applyCachePolicy retains authentication cache precedence.
func applyCachePolicy(headers http.Header, request *http.Request, status int) {
	path := request.URL.EscapedPath()
	_, hasAuthorization := request.Header["Authorization"]
	_, hasCookie := sessionCookie(request.Header)
	switch {
	case path == "/api/auth" || strings.HasPrefix(path, "/api/auth/"):
		headers.Set("Pragma", "no-cache")
		headers.Set("Cache-Control", "no-store")
	case hasAuthorization || hasCookie || status == 401:
		headers.Set("Cache-Control", "private, no-store")
	}
}

func logRequest(request *http.Request, id, route string, status int, duration time.Duration) {
	if (status >= 200 && status < 300 || status == 304) && slices.Contains([]string{"/health/live", "/health/ready"}, route) {
		return
	}
	method := methodLabel(request.Method)
	level, outcome := slog.LevelInfo, "success"
	switch {
	case status >= 500:
		level, outcome = slog.LevelError, "server_error"
	case status >= 400:
		level, outcome = slog.LevelWarn, "client_error"
	case status >= 300:
		outcome = "redirect"
	}
	slog.Log(request.Context(), level, "http.request.completed", "event", "http.request.completed", "component", "http", "request_id", id, "method", method, "route", route, "status", status, "outcome", outcome, "duration_ms", duration.Milliseconds())
}

func methodLabel(method string) string {
	if !slices.Contains([]string{"GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH"}, method) {
		method = "OTHER"
	}
	return method
}
