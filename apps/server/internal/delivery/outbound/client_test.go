package outbound

import (
	"bufio"
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func localClient(server *httptest.Server) *Client {
	client := New(time.Second)
	client.isPrivateAllowed = true
	client.tlsConfig = server.Client().Transport.(*http.Transport).TLSClientConfig.Clone()
	return client
}

func TestMalformedRedirectAndOpaquePathPreserveWireContract(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Accept") != "*/*" || request.Header.Get("User-Agent") != "" {
			t.Errorf("wire contract: path=%q accept=%q agent=%q", request.RequestURI, request.Header.Get("Accept"), request.Header.Get("User-Agent"))
		}
		response.Header().Set("Location", "http://[")
		response.WriteHeader(302)
	}))
	defer server.Close()
	response, err := localClient(server).PostJson(context.Background(), server.URL, nil, map[string]any{}, time.Second)
	if err != nil || response.StatusCode != 302 {
		t.Fatalf("original redirect status lost: %+v %v", response, err)
	}
	listener, err := tls.Listen("tcp", "127.0.0.1:0", server.TLS.Clone())
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	requestLines := make(chan string, 1)
	go func() {
		connection, err := listener.Accept()
		if err != nil {
			requestLines <- "accept failed"
			return
		}
		defer connection.Close()
		connection.SetDeadline(time.Now().Add(time.Second))
		reader := bufio.NewReader(connection)
		line, _ := reader.ReadString('\n')
		requestLines <- line
		for {
			header, err := reader.ReadString('\n')
			if err != nil || header == "\r\n" {
				break
			}
		}
		fmt.Fprint(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
	}()
	response, err = localClient(server).PostJson(context.Background(), "https://"+listener.Addr().String()+"/%GG", nil, map[string]any{}, time.Second)
	if err != nil || response.StatusCode != 200 {
		t.Fatalf("opaque URL: %v %d", err, response.StatusCode)
	}
	if line := <-requestLines; line != "POST /%GG HTTP/1.1\r\n" {
		t.Fatalf("opaque path changed: %q", line)
	}
}

func TestInvalidHeadersFailBeforeDnsAndNeverBecomeRetryableTimeout(t *testing.T) {
	client := New(time.Second)
	var lookups atomic.Int32
	client.lookup = func(context.Context, string, string) ([]netip.Addr, error) {
		lookups.Add(1)
		return nil, errors.New("must not resolve")
	}
	_, err := client.PostJson(context.Background(), "https://invalid-header.example", http.Header{"Authorization": {"Bearer\nsecret"}}, map[string]any{}, time.Second)
	if err != RequestFailed || lookups.Load() != 0 {
		t.Fatalf("builder error reached network: %v %d", err, lookups.Load())
	}
}

func TestConnectionDeadlineIncludesDnsAndDoesNotLimitResponseBody(t *testing.T) {
	client := New(time.Second)
	client.connectTimeout = 35 * time.Millisecond
	finished := make(chan struct{})
	client.lookup = func(context.Context, string, string) ([]netip.Addr, error) {
		<-finished
		return nil, errors.New("released")
	}
	started := time.Now()
	_, err := client.PostJson(context.Background(), "https://slow-dns.example", nil, map[string]any{}, time.Second)
	close(finished)
	if err != TimedOut || time.Since(started) > 500*time.Millisecond {
		t.Fatalf("DNS escaped connect deadline: %v %v", err, time.Since(started))
	}
	server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		time.Sleep(100 * time.Millisecond)
		response.Header().Set("Content-Type", "application/json")
		fmt.Fprint(response, "{}")
	}))
	defer server.Close()
	client = localClient(server)
	client.connectTimeout = 70 * time.Millisecond
	if _, err := client.PostJson(context.Background(), server.URL, nil, map[string]any{}, time.Second); err != nil {
		t.Fatalf("connect deadline leaked into response: %v", err)
	}
}

func TestDualStackFallbackEscapesUnreachablePreferredFamily(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	preferredStopped := make(chan struct{})
	peer, expected := net.Pipe()
	defer peer.Close()
	dial := func(ctx context.Context, network, address string) (net.Conn, error) {
		if strings.HasPrefix(address, "[") {
			<-ctx.Done()
			close(preferredStopped)
			return nil, ctx.Err()
		}
		return expected, nil
	}
	started := time.Now()
	connection, err := dialResolved(ctx, "tcp", "443", []netip.Addr{netip.MustParseAddr("2001:4860:4860::8888"), netip.MustParseAddr("8.8.8.8")}, dial)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if connection != expected || time.Since(started) < 290*time.Millisecond {
		t.Fatal("fallback did not honor family delay")
	}
	select {
	case <-preferredStopped:
	case <-time.After(time.Second):
		t.Fatal("losing dial leaked")
	}
}

func TestSameFamilyPreservesFirstConnectionErrorClassification(t *testing.T) {
	for _, failures := range [][]error{{context.DeadlineExceeded, errors.New("refused")}, {errors.New("refused"), context.DeadlineExceeded}} {
		attempt := 0
		_, err := dialResolved(context.Background(), "tcp", "443", []netip.Addr{netip.MustParseAddr("8.8.8.8"), netip.MustParseAddr("1.1.1.1")}, func(context.Context, string, string) (net.Conn, error) {
			failure := failures[attempt]
			attempt++
			return nil, failure
		})
		if err != failures[0] || attempt != 2 {
			t.Fatalf("first error replaced: %v count=%d", err, attempt)
		}
	}
}

func TestOutboundKeepsStatusWithoutReadingErrorBodyOrFollowingRedirect(t *testing.T) {
	for _, status := range []int{302, 429, 503} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			var requests atomic.Int32
			server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
				requests.Add(1)
				response.Header().Set("Location", "https://should-not-resolve.invalid")
				response.Header().Set("Content-Encoding", "gzip")
				response.Header().Set("Content-Type", "text/html")
				response.Header().Set("Retry-After", "+7")
				response.Header().Set("X-Request-Id", "unsafe id")
				response.Header().Set("Request-Id", "safe:1")
				response.WriteHeader(status)
				response.(http.Flusher).Flush()
				<-request.Context().Done()
			}))
			defer server.Close()
			started := time.Now()
			result, err := localClient(server).PostJson(context.Background(), server.URL, nil, map[string]any{}, time.Second)
			if err != nil || result.StatusCode != status || result.Body != nil || result.RequestId == nil || *result.RequestId != "safe:1" || result.RetryAfterSeconds == nil || *result.RetryAfterSeconds != 7 {
				t.Fatalf("status metadata: %+v %v", result, err)
			}
			if time.Since(started) > 500*time.Millisecond || requests.Load() != 1 {
				t.Fatal("error body read or redirect followed")
			}
		})
	}
}

func TestSuccessBodyTimeoutIsNotRetryableRequestTimeout(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Content-Type", "application/json")
		response.WriteHeader(200)
		response.(http.Flusher).Flush()
		<-request.Context().Done()
	}))
	defer server.Close()
	_, err := localClient(server).PostJson(context.Background(), server.URL, nil, map[string]any{}, 80*time.Millisecond)
	if err != RequestFailed {
		t.Fatalf("body timeout classification: %v", err)
	}
}

func TestOutboundSuccessHeaderAndBodyBoundaries(t *testing.T) {
	tests := []struct {
		name, encoding, contentType, body string
		length                            string
		expected                          error
	}{
		{"gzip", "gzip", "text/plain", "{}", "", UnsupportedContentEncoding},
		{"missing-type", "", "", "{}", "", UnexpectedContentType},
		{"declared-size", "", "application/json", "{}", "1000", ResponseTooLarge},
		{"stream-size", "", "application/json", strings.Repeat(" ", 65), "", ResponseTooLarge},
		{"invalid-json", "", "application/json", "private-sentinel", "", InvalidJson},
		{"identity", "identity", "application/vendor+json; charset=utf-8", `{"id":9223372036854775807}`, "", nil},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
				if request.Header.Get("Accept-Encoding") != "" {
					t.Error("worker must not request compression")
				}
				if test.encoding != "" {
					response.Header().Set("Content-Encoding", test.encoding)
				}
				response.Header()["Content-Type"] = []string{test.contentType}
				if test.length != "" {
					response.Header().Set("Content-Length", test.length)
				}
				response.WriteHeader(200)
				response.(http.Flusher).Flush()
				fmt.Fprint(response, test.body)
			}))
			defer server.Close()
			client := localClient(server)
			client.maximum = 64
			_, err := client.PostJson(context.Background(), server.URL, nil, map[string]any{}, time.Second)
			if err != test.expected {
				t.Fatalf("got %v want %v", err, test.expected)
			}
		})
	}
}

func TestUrlAndAddressPolicyRejectBeforeDial(t *testing.T) {
	client := New(time.Second)
	for _, value := range []string{"http://example.com/", "https://@example.com/", "https://user:secret@example.com/", "https://example.com:0/", "https://example.com/?", "https://example.com/#", "https://127.0.0.1/", "https://2130706433/", "https://[::ffff:127.0.0.1]/"} {
		if _, err := client.ValidateUrl(value); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
	for _, value := range []string{"0.1.2.3", "10.0.0.1", "100.64.0.1", "127.0.0.1", "169.254.1.1", "172.16.0.1", "192.168.1.1", "192.0.0.9", "192.0.2.1", "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "240.0.0.1", "::1", "fc00::1", "2001:db8::1", "2001:1::1", "2002::1", "3fff::1", "2620:4f:8000::1"} {
		if IsPublicAddress(netip.MustParseAddr(value)) {
			t.Errorf("public %s", value)
		}
	}
	for _, value := range []string{"8.8.8.8", "1.1.1.1", "2001:4860:4860::8888", "::ffff:8.8.8.8", "::8.8.8.8"} {
		if !IsPublicAddress(netip.MustParseAddr(value)) {
			t.Errorf("blocked %s", value)
		}
	}
	client.lookup = func(context.Context, string, string) ([]netip.Addr, error) {
		return []netip.Addr{netip.MustParseAddr("8.8.8.8"), netip.MustParseAddr("127.0.0.1")}, nil
	}
	if _, err := client.PostJson(context.Background(), "https://mixed.example/", nil, map[string]any{}, time.Second); err != DisallowedAddress {
		t.Fatalf("mixed DNS: %v", err)
	}
}

func TestTimedOutDnsStillHoldsOneOfEightLookupSlots(t *testing.T) {
	client := New(time.Second)
	var active, maximum atomic.Int32
	release := make(chan struct{})
	var workers sync.WaitGroup
	client.lookup = func(context.Context, string, string) ([]netip.Addr, error) {
		count := active.Add(1)
		for old := maximum.Load(); count > old && !maximum.CompareAndSwap(old, count); old = maximum.Load() {
		}
		<-release
		active.Add(-1)
		return []netip.Addr{netip.MustParseAddr("8.8.8.8")}, nil
	}
	for range 20 {
		workers.Add(1)
		go func() {
			defer workers.Done()
			ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
			defer cancel()
			if _, err := client.resolve(ctx, "slow.example"); err != TimedOut {
				t.Error(err)
			}
		}()
	}
	workers.Wait()
	if maximum.Load() != 8 || active.Load() != 8 {
		t.Fatalf("unbounded lookups: maximum=%d active=%d", maximum.Load(), active.Load())
	}
	close(release)
	deadline := time.Now().Add(time.Second)
	for len(dnsSlots) > 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if len(dnsSlots) != 0 {
		t.Fatal("lookup slots not released")
	}
}
