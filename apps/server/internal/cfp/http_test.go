package cfp

import (
	"compress/gzip"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func publicFixture(t *testing.T, handler http.HandlerFunc) (*HttpTransport, SourceConfig) {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	transport := NewHttpTransport()
	transport.lookup = func(string) ([]netip.Addr, error) { return []netip.Addr{netip.MustParseAddr("8.8.8.8")}, nil }
	transport.dial = func(ctx context.Context, network, address string) (net.Conn, error) {
		if address != "8.8.8.8:80" {
			return nil, fmt.Errorf("unexpected checked address %s", address)
		}
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	}
	t.Cleanup(transport.Close)
	return transport, SourceConfig{DiscoveryUrl: "http://publisher.example/cfp", AllowedUrls: []UrlRule{{Host: "publisher.example", PathPrefix: "/cfp"}}}
}

func TestHttpRedirectBoundariesAndWireBehavior(t *testing.T) {
	for _, scenario := range []struct {
		name, location string
		status, calls  int
		expected       error
	}{
		{"relative", "/cfp/final", 302, 2, nil},
		{"nonstandard-3xx", "/cfp/final", 399, 2, nil},
		{"literal-percent-reaches-server", "/cfp/%", 302, 1, SourceError{Kind: "http_status", Status: 400}},
		{"empty-location", "", 302, 6, ErrDisallowedUrl},
		{"sibling-prefix", "/cfps", 302, 1, ErrDisallowedUrl},
		{"private-literal", "http://127.0.0.1/cfp", 302, 1, ErrDisallowedUrl},
		{"credentials", "http://user:pass@publisher.example/cfp", 302, 1, ErrDisallowedUrl},
		{"status-no-retry", "", 503, 1, SourceError{Kind: "http_status", Status: 503}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			var calls atomic.Int32
			transport, config := publicFixture(t, func(writer http.ResponseWriter, request *http.Request) {
				calls.Add(1)
				if request.Header.Get("User-Agent") != "LitRadar/CFP (+original publisher announcements)" || request.Header.Get("Accept-Encoding") != "gzip" || request.Header.Get("Cookie") != "" {
					t.Error("request headers changed", request.Header)
				}
				if request.RequestURI == "/cfp" {
					writer.Header().Set("Location", scenario.location)
					writer.Header().Set("Set-Cookie", "session=must-not-forward")
					writer.WriteHeader(scenario.status)
					return
				}
				writer.Header().Set("Content-Encoding", "gzip")
				compressed := gzip.NewWriter(writer)
				compressed.Write([]byte("complete publisher text"))
				compressed.Close()
			})
			document, err := transport.FetchBytes(context.Background(), config, config.DiscoveryUrl, time.Now().Add(3*time.Second))
			if !errors.Is(err, scenario.expected) || int(calls.Load()) != scenario.calls {
				t.Fatalf("error=%v calls=%d", err, calls.Load())
			}
			if err == nil && string(document.Bytes) != "complete publisher text" {
				t.Fatalf("body=%q", document.Bytes)
			}
		})
	}
}

func TestHttpBodyBoundsAndNoPartialRetry(t *testing.T) {
	for _, scenario := range []string{"decoded-overflow", "declared-overflow", "partial", "exact-bound"} {
		t.Run(scenario, func(t *testing.T) {
			var calls atomic.Int32
			transport, config := publicFixture(t, func(writer http.ResponseWriter, request *http.Request) {
				calls.Add(1)
				switch scenario {
				case "declared-overflow":
					writer.Header().Set("Content-Length", fmt.Sprint(MaxPageBytes+1))
				case "partial":
					writer.Header().Set("Content-Length", "100")
					io.WriteString(writer, "short")
				default:
					writer.Header().Set("Content-Encoding", "gzip")
					compressed := gzip.NewWriter(writer)
					length := MaxPageBytes
					if scenario == "decoded-overflow" {
						length++
					}
					io.WriteString(compressed, strings.Repeat("a", length))
					compressed.Close()
				}
			})
			document, err := transport.FetchBytes(context.Background(), config, config.DiscoveryUrl, time.Now().Add(5*time.Second))
			expected := error(ErrTooLarge)
			if scenario == "partial" {
				expected = ErrRequest
			}
			if scenario == "exact-bound" {
				expected = nil
				if len(document.Bytes) != MaxPageBytes {
					t.Fatal("truncated allowed body")
				}
			}
			if !errors.Is(err, expected) || calls.Load() != 1 {
				t.Fatalf("error=%v calls=%d", err, calls.Load())
			}
		})
	}
}

func TestHttpPublicDnsAndLifetime(t *testing.T) {
	transport := NewHttpTransport()
	defer transport.Close()
	for _, addresses := range [][]netip.Addr{nil, {netip.MustParseAddr("8.8.8.8"), netip.MustParseAddr("127.0.0.1")}, {netip.MustParseAddr("::ffff:127.0.0.1")}} {
		transport.lookup = func(string) ([]netip.Addr, error) { return addresses, nil }
		if _, err := transport.resolve(context.Background(), "publisher.example"); !errors.Is(err, ErrDisallowedUrl) {
			t.Fatal(err)
		}
	}
	started, release := make(chan struct{}, 8), make(chan struct{})
	defer close(release)
	transport.lookup = func(string) ([]netip.Addr, error) {
		started <- struct{}{}
		<-release
		return nil, fmt.Errorf("fixture")
	}
	for index := 0; index < cap(dnsSlots); index++ {
		ctx, cancel := context.WithCancel(context.Background())
		finished := make(chan error, 1)
		go func() { _, err := transport.resolve(ctx, "publisher.example"); finished <- err }()
		<-started
		cancel()
		if err := <-finished; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
	defer cancel()
	if _, err := transport.resolve(ctx, "publisher.example"); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatal("OS lookup slots released before completion", err)
	}
}

func TestHttpConnectionCancellationAndDualStack(t *testing.T) {
	transport := NewHttpTransport()
	defer transport.Close()
	transport.lookup = func(string) ([]netip.Addr, error) {
		return []netip.Addr{netip.MustParseAddr("2001:4860:4860::8888"), netip.MustParseAddr("8.8.8.8")}, nil
	}
	var ipv4 atomic.Bool
	transport.dial = func(ctx context.Context, network, address string) (net.Conn, error) {
		if address == "8.8.8.8:80" {
			ipv4.Store(true)
			first, second := net.Pipe()
			second.Close()
			return first, nil
		}
		<-ctx.Done()
		return nil, ctx.Err()
	}
	ctx, cancel := context.WithTimeout(context.Background(), 1500*time.Millisecond)
	defer cancel()
	connection, err := transport.connect(ctx, "tcp", "publisher.example:80", false)
	if err != nil || !ipv4.Load() {
		t.Fatal("IPv6 black hole prevented IPv4", err)
	}
	connection.Close()
	requestCtx, requestCancel := context.WithCancel(context.Background())
	requestCancel()
	dialCtx := context.WithValue(context.Background(), requestContextKey{}, requestCtx)
	transport.dial = func(ctx context.Context, _, _ string) (net.Conn, error) { <-ctx.Done(); return nil, ctx.Err() }
	started := time.Now()
	if _, err := transport.connect(dialCtx, "tcp", "publisher.example:80", false); err == nil || time.Since(started) > time.Second {
		t.Fatal("detached transport context lost request cancellation", err)
	}
}
