// Package outbound enforces the notification worker's bounded request-time network policy.
package outbound

import (
	"context"
	"crypto/tls"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptrace"
	"net/netip"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/sources"
	platform "github.com/QianFuv/LitRadar/internal/platform/transport"
	"github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
	"golang.org/x/net/http/httpguts"
)

const MaxResponseBytes = 2 * 1024 * 1024

var dnsSlots = make(chan struct{}, 8)

// Error identifies a safe network failure without carrying headers, URLs or upstream bodies.
type Error string

const (
	InvalidUrl                 Error = "Outbound request URL is invalid"
	HttpsRequired              Error = "Outbound requests require HTTPS"
	HostRequired               Error = "Outbound request URL requires a host"
	DnsResolutionFailed        Error = "Outbound endpoint DNS resolution failed"
	DisallowedAddress          Error = "Outbound endpoint resolved to a disallowed address"
	ConnectFailed              Error = "Outbound endpoint connection failed"
	TimedOut                   Error = "Outbound request timed out"
	RequestFailed              Error = "Outbound request failed"
	UnsupportedContentEncoding Error = "Outbound response uses an unsupported content encoding"
	UnexpectedContentType      Error = "Outbound response must use a JSON content type"
	ResponseTooLarge           Error = "Outbound response exceeded the size limit"
	InvalidJson                Error = "Outbound response contained invalid JSON"
)

func (err Error) Error() string { return string(err) }

// Response contains only validated success JSON and safe response metadata.
type Response struct {
	StatusCode        int
	RequestId         *string
	RetryAfterSeconds *uint64
	Body              any
}

// String prevents ordinary logs from expanding upstream content.
func (response Response) String() string { return "OutboundResponse([REDACTED])" }

// GoString prevents Go-syntax logs from expanding upstream content.
func (response Response) GoString() string { return response.String() }

// LogValue prevents structured logs from expanding upstream content.
func (response Response) LogValue() slog.Value { return slog.StringValue(response.String()) }

// Client constructs a fresh pinned, proxy-free connection pool for each request.
type Client struct {
	connectTimeout                  time.Duration
	maximum                         int64
	isHttpAllowed, isPrivateAllowed bool
	lookup                          func(context.Context, string, string) ([]netip.Addr, error)
	tlsConfig                       *tls.Config
}

// New configures production HTTPS and public-address restrictions.
func New(timeout time.Duration) *Client {
	return &Client{connectTimeout: min(10*time.Second, max(time.Second, timeout)), maximum: MaxResponseBytes, lookup: net.DefaultResolver.LookupNetIP}
}

// ValidateUrl checks parsed and raw-authority boundaries before any network operation.
func (client *Client) ValidateUrl(value string) (*whatwg.Url, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return nil, InvalidUrl
	}
	if location.Scheme() != "https" && !(client.isHttpAllowed && location.Scheme() == "http") {
		return nil, HttpsRequired
	}
	_, authority, _ := strings.Cut(value, "://")
	if index := strings.IndexAny(authority, "/?#"); index >= 0 {
		authority = authority[:index]
	}
	if strings.Contains(authority, "@") || location.Username() != "" || location.Password() != "" || location.Port() == "0" || strings.Contains(location.Href(true), "?") || location.Href(false) != location.Href(true) {
		return nil, InvalidUrl
	}
	if location.Hostname() == "" {
		return nil, HostRequired
	}
	if address, err := netip.ParseAddr(strings.Trim(location.Hostname(), "[]")); err == nil && !client.isPrivateAllowed && !IsPublicAddress(address) {
		return nil, DisallowedAddress
	}
	return location, nil
}

func (client *Client) resolve(ctx context.Context, host string) ([]netip.Addr, error) {
	if address, err := netip.ParseAddr(host); err == nil {
		return []netip.Addr{address}, nil
	}
	select {
	case dnsSlots <- struct{}{}:
	case <-ctx.Done():
		return nil, TimedOut
	}
	type lookupResult struct {
		addresses []netip.Addr
		err       error
	}
	completed := make(chan lookupResult, 1)
	go func() {
		defer func() { <-dnsSlots }()
		addresses, err := client.lookup(context.Background(), "ip", host)
		completed <- lookupResult{addresses, err}
	}()
	select {
	case <-ctx.Done():
		return nil, TimedOut
	case result := <-completed:
		if result.err != nil || len(result.addresses) == 0 {
			return nil, DnsResolutionFailed
		}
		for _, address := range result.addresses {
			if !client.isPrivateAllowed && !IsPublicAddress(address) {
				return nil, DisallowedAddress
			}
		}
		return result.addresses, nil
	}
}

// PostJson enforces a total timeout including DNS, forbids redirects and compression, and never reads non-success bodies.
func (client *Client) PostJson(ctx context.Context, value string, headers http.Header, body any, timeout time.Duration) (Response, error) {
	location, err := client.ValidateUrl(value)
	if err != nil {
		return Response{}, err
	}
	ctx, cancel := context.WithTimeout(ctx, max(time.Millisecond, timeout))
	defer cancel()
	encoded, err := sources.Json(body)
	if err != nil {
		return Response{}, RequestFailed
	}
	headers = headers.Clone()
	if headers == nil {
		headers = make(http.Header)
	}
	if headers.Get("Content-Type") == "" {
		headers.Set("Content-Type", "application/json")
	}
	if _, exists := headers["Accept"]; !exists {
		headers.Set("Accept", "*/*")
	}
	if _, exists := headers["User-Agent"]; !exists {
		headers.Set("User-Agent", "")
	}
	for name, values := range headers {
		if !httpguts.ValidHeaderFieldName(name) {
			return Response{}, RequestFailed
		}
		for _, value := range values {
			if !httpguts.ValidHeaderFieldValue(value) {
				return Response{}, RequestFailed
			}
		}
	}
	connection, err := platform.New("")
	if err != nil {
		return Response{}, RequestFailed
	}
	defer connection.CloseIdleConnections()
	connection.DisableCompression = true
	connection.TLSClientConfig = &tls.Config{}
	if client.tlsConfig != nil {
		connection.TLSClientConfig = client.tlsConfig.Clone()
	}
	connection.DialContext = func(dialContext context.Context, network, address string) (net.Conn, error) {
		return client.connect(dialContext, ctx, network, address, nil)
	}
	connection.DialTLSContext = func(dialContext context.Context, network, address string) (net.Conn, error) {
		return client.connect(dialContext, ctx, network, address, connection.TLSClientConfig)
	}
	var hasConnection atomic.Bool
	ctx = httptrace.WithClientTrace(ctx, &httptrace.ClientTrace{GotConn: func(httptrace.GotConnInfo) { hasConnection.Store(true) }})
	response, err := transport.RedirectSequence(ctx, connection, nil, transport.Proxy{}, http.MethodPost, location.Href(false), string(encoded), headers, nil)
	if err != nil {
		var policyError Error
		if errors.As(err, &policyError) {
			return Response{}, policyError
		}
		if errors.Is(err, transport.ErrRequestBuild) {
			return Response{}, RequestFailed
		}
		if errors.Is(ctx.Err(), context.Canceled) {
			return Response{}, RequestFailed
		}
		var networkError net.Error
		if errors.Is(ctx.Err(), context.DeadlineExceeded) || errors.As(err, &networkError) && networkError.Timeout() {
			return Response{}, TimedOut
		}
		if !hasConnection.Load() {
			return Response{}, ConnectFailed
		}
		return Response{}, RequestFailed
	}
	defer response.Body.Close()
	result := Response{StatusCode: response.StatusCode, RequestId: safeRequestId(response.Header), RetryAfterSeconds: retryAfter(response.Header)}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return result, nil
	}
	if err := validateHeaders(response.Header, client.maximum); err != nil {
		return Response{}, err
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, client.maximum+1))
	if err != nil || ctx.Err() != nil {
		return Response{}, RequestFailed
	}
	if int64(len(data)) > client.maximum {
		return Response{}, ResponseTooLarge
	}
	parsed, err := transport.ParseJson(data)
	if err != nil {
		return Response{}, InvalidJson
	}
	result.Body = parsed
	return result, nil
}

func validateHeaders(headers http.Header, maximum int64) error {
	for _, encoding := range headers.Values("Content-Encoding") {
		if isHeaderText(encoding) && !strings.EqualFold(encoding, "identity") {
			return UnsupportedContentEncoding
		}
	}
	contentType := headers.Get("Content-Type")
	if !isHeaderText(contentType) {
		contentType = ""
	}
	contentType, _, _ = strings.Cut(contentType, ";")
	contentType = strings.ToLower(strings.TrimSpace(contentType))
	if contentType != "application/json" && !strings.HasSuffix(contentType, "+json") {
		return UnexpectedContentType
	}
	length := strings.TrimPrefix(headers.Get("Content-Length"), "+")
	if count, err := strconv.ParseUint(length, 10, 64); err == nil && count > uint64(maximum) {
		return ResponseTooLarge
	}
	return nil
}

func isHeaderText(value string) bool {
	for _, character := range []byte(value) {
		if character != 9 && (character < 32 || character >= 127) {
			return false
		}
	}
	return true
}

func safeRequestId(headers http.Header) *string {
	for _, name := range []string{"x-request-id", "request-id", "cf-ray"} {
		value := headers.Get(name)
		if value == "" || len(value) > 128 {
			continue
		}
		isSafe := true
		for _, character := range []byte(value) {
			if !(character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("-_.:", rune(character))) {
				isSafe = false
				break
			}
		}
		if isSafe {
			return &value
		}
	}
	return nil
}

func retryAfter(headers http.Header) *uint64 {
	value := headers.Get("Retry-After")
	if !isHeaderText(value) {
		return nil
	}
	value = strings.TrimPrefix(strings.TrimSpace(value), "+")
	seconds, err := strconv.ParseUint(value, 10, 64)
	if err != nil {
		return nil
	}
	return &seconds
}
