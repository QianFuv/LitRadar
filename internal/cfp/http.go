package cfp

import (
	"bytes"
	"context"
	"crypto/tls"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/delivery/outbound"
	"github.com/QianFuv/LitRadar/internal/platform/textdecode"
	wire "github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

type HttpDocument struct {
	FinalUrl, ContentType string
	Bytes                 []byte
}
type HttpTransport struct {
	client *http.Client
	lookup func(string) ([]netip.Addr, error)
	dial   func(context.Context, string, string) (net.Conn, error)
}
type requestContextKey struct{}

var dnsSlots = make(chan struct{}, 8)

// NewHttpTransport bypasses environment proxies and binds each connection to checked public addresses.
func NewHttpTransport() *HttpTransport {
	result := &HttpTransport{dial: (&net.Dialer{}).DialContext, lookup: func(host string) ([]netip.Addr, error) {
		return net.DefaultResolver.LookupNetIP(context.Background(), "ip", host)
	}}
	transport := &http.Transport{Proxy: nil, ForceAttemptHTTP2: true, MaxIdleConns: 32, IdleConnTimeout: 90 * time.Second, TLSHandshakeTimeout: 8 * time.Second, DisableCompression: true}
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return result.connect(ctx, network, address, false)
	}
	transport.DialTLSContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return result.connect(ctx, network, address, true)
	}
	result.client = &http.Client{Transport: &wire.ClientTransport{Transport: transport}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	return result
}
func (transport *HttpTransport) Close() { transport.client.CloseIdleConnections() }

// resolve preserves bounded lookup ownership and cancellation before public address admission.
func (transport *HttpTransport) resolve(ctx context.Context, host string) ([]netip.Addr, error) {
	if address, err := netip.ParseAddr(host); err == nil {
		if !outbound.IsPublicAddress(address) {
			return nil, ErrDisallowedUrl
		}
		return []netip.Addr{address}, nil
	}
	select {
	case dnsSlots <- struct{}{}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	type resolution struct {
		addresses []netip.Addr
		err       error
	}
	completed := make(chan resolution, 1)
	go func() { addresses, err := transport.lookup(host); <-dnsSlots; completed <- resolution{addresses, err} }()
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case result := <-completed:
		return publicResolvedAddresses(result.addresses, result.err)
	}
}
func (transport *HttpTransport) connect(ctx context.Context, network, address string, isTls bool) (net.Conn, error) {
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
	if !isTls {
		return connection, nil
	}
	secured := tls.Client(connection, &tls.Config{ServerName: host, MinVersion: tls.VersionTLS12, NextProtos: []string{"h2", "http/1.1"}})
	if err = secured.HandshakeContext(ctx); err != nil {
		connection.Close()
		return nil, err
	}
	return secured, nil
}

func validateUrl(config SourceConfig, location *whatwg.Url) error {
	if !config.PermitsUrl(location) {
		return ErrDisallowedUrl
	}
	if address, err := netip.ParseAddr(strings.Trim(location.Hostname(), "[]")); err == nil && !outbound.IsPublicAddress(address) {
		return ErrDisallowedUrl
	}
	return nil
}

// FetchBytes checks every hop and bounds the decompressed body without retrying partial reads.
func (transport *HttpTransport) FetchBytes(ctx context.Context, config SourceConfig, value string, deadline time.Time) (HttpDocument, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return HttpDocument{}, ErrDisallowedUrl
	}
	for redirect := 0; redirect <= 5; redirect++ {
		if err = validateUrl(config, location); err != nil {
			return HttpDocument{}, err
		}
		response, release, err := transport.fetchResponse(ctx, location, deadline)
		if err != nil {
			return HttpDocument{}, err
		}
		closeResponse := func() { response.Body.Close(); release() }
		if isRedirectStatus(response.StatusCode) {
			location, err = redirectLocation(response, location, redirect, closeResponse)
			if err != nil {
				return HttpDocument{}, err
			}
			continue
		}
		return readHttpDocument(response, location, deadline, closeResponse)
	}
	return HttpDocument{}, ErrDisallowedUrl
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
	text, err := textdecode.Html(body, contentType)
	if err != nil {
		return "", ErrEncoding
	}
	return text, nil
}

// Fetch supplies HTML/text only; the live worker owns PDF conversion and rendered fallback.
func (transport *HttpTransport) Fetch(ctx context.Context, config SourceConfig, value string, deadline time.Time) (Document, error) {
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
	return Document{FinalUrl: response.FinalUrl, Text: text, Format: "html"}, nil
}

// publicResolvedAddresses requires a successful nonempty entirely public lookup result.
func publicResolvedAddresses(addresses []netip.Addr, err error) ([]netip.Addr, error) {
	if err != nil {
		return nil, err
	}
	if len(addresses) == 0 {
		return nil, ErrDisallowedUrl
	}
	for _, address := range addresses {
		if !outbound.IsPublicAddress(address) {
			return nil, ErrDisallowedUrl
		}
	}
	return addresses, nil
}

// fetchResponse retries only request acquisition and returns cancellation ownership with a response.
func (transport *HttpTransport) fetchResponse(ctx context.Context, location *whatwg.Url, deadline time.Time) (*http.Response, context.CancelFunc, error) {
	var err error
	var response *http.Response
	var release context.CancelFunc
	for attempt := 0; attempt < 2; attempt++ {
		remaining := time.Until(deadline)
		if remaining <= 0 {
			return nil, nil, ErrDeadline
		}
		requestCtx, cancel := context.WithTimeout(ctx, min(remaining, 20*time.Second))
		dialCtx := context.WithValue(requestCtx, requestContextKey{}, requestCtx)
		headers := http.Header{"User-Agent": {"LitRadar/CFP (+original publisher announcements)"}, "Accept": {"text/html,application/pdf,text/plain;q=0.8"}}
		response, err = wire.RedirectSequence(dialCtx, transport.client.Transport, nil, wire.Proxy{}, http.MethodGet, location.Href(false), "", headers, nil)
		if err == nil {
			release = cancel
			break
		}
		cancel()
		if errors.Is(err, ErrDisallowedUrl) {
			return nil, nil, ErrDisallowedUrl
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

// isRedirectStatus recognizes the original entire 3xx range.
func isRedirectStatus(status int) bool { return status >= 300 && status < 400 }

// redirectLocation closes the response before admitting the first Location header and parsed target.
func redirectLocation(response *http.Response, location *whatwg.Url, redirect int, closeResponse func()) (*whatwg.Url, error) {
	if redirect == 5 {
		closeResponse()
		return nil, ErrDisallowedUrl
	}
	locations, hasLocation := response.Header["Location"]
	target := ""
	if len(locations) > 0 {
		target = locations[0]
	}
	closeResponse()
	if !hasLocation || len(locations) == 0 || !isHeaderText(target) {
		return nil, ErrDisallowedUrl
	}
	location, err := whatwg.NewParser().ParseRef(location.Href(false), target)
	if err != nil {
		return nil, ErrDisallowedUrl
	}
	return location, nil
}

// readHttpDocument preserves status, declared length, bounded read, cleanup and deadline error priority.
func readHttpDocument(response *http.Response, location *whatwg.Url, deadline time.Time, closeResponse func()) (HttpDocument, error) {
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		closeResponse()
		return HttpDocument{}, SourceError{Kind: "http_status", Status: response.StatusCode}
	}
	if response.ContentLength > MaxPageBytes {
		closeResponse()
		return HttpDocument{}, ErrTooLarge
	}
	contentType := response.Header.Get("Content-Type")
	if !isHeaderText(contentType) {
		contentType = ""
	}
	body, readErr := io.ReadAll(io.LimitReader(response.Body, MaxPageBytes+1))
	closeResponse()
	if readErr != nil {
		if !time.Now().Before(deadline) {
			return HttpDocument{}, ErrDeadline
		}
		return HttpDocument{}, ErrRequest
	}
	if len(body) > MaxPageBytes {
		return HttpDocument{}, ErrTooLarge
	}
	if !time.Now().Before(deadline) {
		return HttpDocument{}, ErrDeadline
	}
	return HttpDocument{FinalUrl: location.Href(false), ContentType: contentType, Bytes: body}, nil
}
