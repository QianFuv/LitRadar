package api

import (
	"crypto/rand"
	"fmt"
	"log/slog"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"
)

// ServeHTTP applies the public security, correlation, CORS and cache contract.
func (handler *Handler) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	started := time.Now()
	var identifier [16]byte
	_, _ = rand.Read(identifier[:])
	identifier[6] = identifier[6]&0x0f | 0x40
	identifier[8] = identifier[8]&0x3f | 0x80
	id := fmt.Sprintf("%x-%x-%x-%x-%x", identifier[:4], identifier[4:6], identifier[6:8], identifier[8:10], identifier[10:])
	request = withRequestId(request, id)
	selected, label, allow := handler.route(request)
	response := &policyResponse{ResponseWriter: writer, isHead: request.Method == "HEAD"}
	isPreflight := request.Method == "OPTIONS"
	redirect := frontendRedirect(request)
	response.hasMatchedHead = redirect == "" && (selected != nil || allow != "")
	response.prepare = func(status int) {
		headers := writer.Header()
		headers.Set("X-Request-Id", id)
		headers.Set("Content-Security-Policy", handler.options.ContentSecurityPolicy)
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
			if redirect == "" {
				applyCachePolicy(headers, request, status)
			}
		}
	}
	switch {
	case isPreflight:
		if allow != "" {
			response.Header().Set("Allow", allow)
		}
		response.WriteHeader(http.StatusOK)
	case redirect != "":
		response.Header().Set("Location", redirect)
		response.WriteHeader(http.StatusPermanentRedirect)
	case selected != nil:
		selected.ServeHTTP(response, request)
	case allow != "":
		response.Header().Set("Allow", allow)
		response.WriteHeader(http.StatusMethodNotAllowed)
	default:
		response.WriteHeader(http.StatusNotFound)
	}
	response.finish()
	logRequest(request, id, label, response.status, time.Since(started))
}

type policyResponse struct {
	http.ResponseWriter
	prepare        func(int)
	status         int
	isHead         bool
	hasMatchedHead bool
	length         int64
}

func (writer *policyResponse) Unwrap() http.ResponseWriter { return writer.ResponseWriter }
func (writer *policyResponse) WriteHeader(status int) {
	if writer.status != 0 {
		return
	}
	writer.status = status
	if !writer.isHead {
		writer.prepare(status)
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

func isBackendPath(path string) bool {
	for _, prefix := range []string{"/api", "/health", "/mcp", "/docs", "/openapi.json"} {
		if path == prefix || strings.HasPrefix(path, prefix+"/") {
			return true
		}
	}
	return false
}
func frontendRedirect(request *http.Request) string {
	path := request.URL.EscapedPath()
	if request.Method != "GET" && request.Method != "HEAD" || !strings.HasSuffix(path, "/") || isBackendPath(path) {
		return ""
	}
	path = strings.TrimRight(path, "/")
	if path == "" {
		return ""
	}
	if request.URL.RawQuery != "" || request.URL.ForceQuery {
		path += "?" + request.URL.RawQuery
	}
	return path
}
func applyCachePolicy(headers http.Header, request *http.Request, status int) {
	path := request.URL.EscapedPath()
	_, hasAuthorization := request.Header["Authorization"]
	_, hasCookie := sessionCookie(request.Header)
	switch {
	case path == "/api/auth" || strings.HasPrefix(path, "/api/auth/"):
		headers.Set("Pragma", "no-cache")
		headers.Set("Cache-Control", "no-store")
	case strings.HasPrefix(path, "/_next/static/") && (status >= 200 && status < 300 || status == 304):
		headers.Set("Cache-Control", "public, max-age=31536000, immutable")
	case hasAuthorization || hasCookie || status == 401:
		headers.Set("Cache-Control", "private, no-store")
	case !isBackendPath(path) && (request.Method == "GET" || request.Method == "HEAD") && (status >= 200 && status < 300 || status == 404 || status == 304):
		headers.Set("Cache-Control", "no-cache")
	}
}
func logRequest(request *http.Request, id, route string, status int, duration time.Duration) {
	if (status >= 200 && status < 300 || status == 304) && slices.Contains([]string{"/health/live", "/health/ready", "static.asset", "static.frontend"}, route) {
		return
	}
	method := request.Method
	if !slices.Contains([]string{"GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH"}, method) {
		method = "OTHER"
	}
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
