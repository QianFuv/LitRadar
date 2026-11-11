package cfp

import (
	"bytes"
	"context"
	"crypto/tls"
	"errors"
	"golang.org/x/sync/semaphore"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/delivery/outbound"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/textdecode"
	wire "github.com/QianFuv/LitRadar/apps/server/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

type HTTPDocument struct {
	FinalURL, ContentType string
	Bytes                 []byte
}
type HTTPTransport struct {
	client *http.Client
	lookup func(string) ([]netip.Addr, error)
	dial   func(context.Context, string, string) (net.Conn, error)
}
type requestContextKey struct{}

const dnsLookupLimit = 8

var dnsSlots = semaphore.NewWeighted(dnsLookupLimit)

// NewHTTPTransport bypasses environment proxies and binds each connection to checked public addresses.
func NewHTTPTransport() *HTTPTransport {
	result := &HTTPTransport{dial: (&net.Dialer{}).DialContext, lookup: func(host string) ([]netip.Addr, error) {
		return net.DefaultResolver.LookupNetIP(context.Background(), "ip", host)
	}}
	transport := &http.Transport{Proxy: nil, ForceAttemptHTTP2: true, MaxIdleConns: 32, IdleConnTimeout: 90 * time.Second, TLSHandshakeTimeout: 8 * time.Second, DisableCompression: false}
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return result.connect(ctx, network, address, false)
	}
	transport.DialTLSContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return result.connect(ctx, network, address, true)
	}
	result.client = &http.Client{Transport: &wire.ClientTransport{Transport: transport}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	return result
}
func (transport *HTTPTransport) Close() { transport.client.CloseIdleConnections() }

// resolve preserves bounded lookup ownership and cancellation before public address admission.
func (transport *HTTPTransport) resolve(ctx context.Context, host string) ([]netip.Addr, error) {
	if address, err := netip.ParseAddr(host); err == nil {
		if !outbound.IsPublicAddress(address) {
			return nil, ErrDisallowedURL
		}
		return []netip.Addr{address}, nil
	}
	if err := dnsSlots.Acquire(ctx, 1); err != nil {
		return nil, err
	}
	type resolution struct {
		addresses []netip.Addr
		err       error
	}
	completed := make(chan resolution, 1)
	go func() {
		addresses, err := transport.lookup(host)
		dnsSlots.Release(1)
		completed <- resolution{addresses, err}
	}()
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case result := <-completed:
		return publicResolvedAddresses(result.addresses, result.err)
	}
}
func (transport *HTTPTransport) connect(ctx context.Context, network, address string, isTLS bool) (net.Conn, error) {
	ctx, cancel := context.WithTimeout(ctx, 8*time.Second)
	defer cancel()
	if requestCtx, ok := ctx.Value(requestContextKey{}).(context.Context); ok {
		stop := context.AfterFunc(requestCtx, cancel)
		defer stop()
	}
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, err
	}
	addresses, err := transport.resolve(ctx, host)
	if err != nil {
		return nil, err
	}
	connection, err := outbound.DialResolved(ctx, network, port, addresses, transport.dial)
	if err != nil {
		return nil, err
	}
	if !isTLS {
		return connection, nil
	}
	secured := tls.Client(connection, &tls.Config{ServerName: host, MinVersion: tls.VersionTLS12, NextProtos: []string{"h2", "http/1.1"}})
	if err = secured.HandshakeContext(ctx); err != nil {
		if err := connection.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "connection.Close", "error", err)
		}
		return nil, err
	}
	return secured, nil
}

func validateURL(config SourceConfig, location *whatwg.Url) error {
	if !config.PermitsURL(location) {
		return ErrDisallowedURL
	}
	if address, err := netip.ParseAddr(strings.Trim(location.Hostname(), "[]")); err == nil && !outbound.IsPublicAddress(address) {
		return ErrDisallowedURL
	}
	return nil
}

// FetchBytes checks every hop and bounds the decompressed body without retrying partial reads.
func (transport *HTTPTransport) FetchBytes(ctx context.Context, config SourceConfig, value string, deadline time.Time) (HTTPDocument, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return HTTPDocument{}, ErrDisallowedURL
	}
	if err := validateURL(config, location); err != nil {
		return HTTPDocument{}, err
	}
	response, release, err := transport.fetchResponse(ctx, config, location, deadline)
	if err != nil {
		return HTTPDocument{}, err
	}
	closeResponse := func() { _ = response.Body.Close(); release() }
	final, err := whatwg.NewParser().Parse(response.Request.URL.String())
	if err != nil {
		closeResponse()
		return HTTPDocument{}, ErrDisallowedURL
	}
	return readHTTPDocument(response, final, deadline, closeResponse)
}

func isHeaderText(value string) bool {
	for _, value := range []byte(value) {
		if value < 32 && value != '\t' || value >= 127 {
			return false
		}
	}
	return true
}

func DecodeBody(body []byte, contentType string) (string, error) {
	if len(body) > MaxPageBytes {
		return "", ErrTooLarge
	}
	text, err := textdecode.HTML(body, contentType)
	if err != nil {
		return "", ErrEncoding
	}
	return text, nil
}

// Fetch supplies HTML/text only; the live worker owns PDF conversion and rendered fallback.
func (transport *HTTPTransport) Fetch(ctx context.Context, config SourceConfig, value string, deadline time.Time) (Document, error) {
	response, err := transport.FetchBytes(ctx, config, value, deadline)
	if err != nil {
		return Document{}, err
	}
	if bytes.HasPrefix(response.Bytes, []byte("%PDF-")) || strings.Contains(strings.ToLower(response.ContentType), "application/pdf") {
		return Document{}, ErrContentType
	}
	text, err := DecodeBody(response.Bytes, response.ContentType)
	if err != nil {
		return Document{}, err
	}
	return Document{FinalURL: response.FinalURL, Text: text, Format: "html"}, nil
}

// publicResolvedAddresses requires a successful nonempty entirely public lookup result.
func publicResolvedAddresses(addresses []netip.Addr, err error) ([]netip.Addr, error) {
	if err != nil {
		return nil, err
	}
	if len(addresses) == 0 {
		return nil, ErrDisallowedURL
	}
	for _, address := range addresses {
		if !outbound.IsPublicAddress(address) {
			return nil, ErrDisallowedURL
		}
	}
	return addresses, nil
}

// fetchResponse retries only request acquisition and returns cancellation ownership with a response.
func (transport *HTTPTransport) fetchResponse(ctx context.Context, config SourceConfig, location *whatwg.Url, deadline time.Time) (*http.Response, context.CancelFunc, error) {
	var err error
	var response *http.Response
	var release context.CancelFunc
	for attempt := range 2 {
		remaining := time.Until(deadline)
		if remaining <= 0 {
			return nil, nil, ErrDeadline
		}
		requestCtx, cancel := context.WithTimeout(ctx, min(remaining, 20*time.Second))
		dialCtx := context.WithValue(requestCtx, requestContextKey{}, requestCtx)
		headers := http.Header{"User-Agent": {"LitRadar/CFP (+original publisher announcements)"}, "Accept": {"text/html,application/pdf,text/plain;q=0.8"}}
		response, err = wire.RedirectSequence(dialCtx, transport.client.Transport, nil, http.MethodGet, location.Href(false), "", headers, func(next string, hops int) error {
			if hops > 5 {
				return ErrDisallowedURL
			}
			location, err := whatwg.NewParser().Parse(next)
			if err != nil {
				return ErrDisallowedURL
			}
			return validateURL(config, location)
		})
		if err == nil {
			release = cancel
			break
		}
		cancel()
		if errors.Is(err, ErrDisallowedURL) {
			return nil, nil, ErrDisallowedURL
		}
		if !time.Now().Before(deadline) {
			return nil, nil, ErrDeadline
		}
		if attempt == 1 {
			return nil, nil, ErrRequest
		}
	}
	if response == nil {
		return nil, nil, ErrRequest
	}
	return response, release, nil
}

// readHTTPDocument preserves status, declared length, bounded read, cleanup and deadline error priority.
func readHTTPDocument(response *http.Response, location *whatwg.Url, deadline time.Time, closeResponse func()) (HTTPDocument, error) {
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		closeResponse()
		return HTTPDocument{}, SourceError{Kind: "http_status", Status: response.StatusCode}
	}
	if response.ContentLength > MaxPageBytes {
		closeResponse()
		return HTTPDocument{}, ErrTooLarge
	}
	contentType := response.Header.Get("Content-Type")
	if !isHeaderText(contentType) {
		contentType = ""
	}
	body, readErr := io.ReadAll(io.LimitReader(response.Body, MaxPageBytes+1))
	closeResponse()
	if readErr != nil {
		if !time.Now().Before(deadline) {
			return HTTPDocument{}, ErrDeadline
		}
		return HTTPDocument{}, ErrRequest
	}
	if len(body) > MaxPageBytes {
		return HTTPDocument{}, ErrTooLarge
	}
	if !time.Now().Before(deadline) {
		return HTTPDocument{}, ErrDeadline
	}
	return HTTPDocument{FinalURL: location.Href(false), ContentType: contentType, Bytes: body}, nil
}
