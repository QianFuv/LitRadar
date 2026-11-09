package outbound

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"reflect"
	"testing"
	"time"
)

func TestOutboundAddressRangeNeighborsAndIpv4Normalization(t *testing.T) {
	cases := map[string]bool{"100.63.255.255": true, "100.64.0.0": false, "100.127.255.255": false, "100.128.0.0": true, "2001:1ff:ffff::1": false, "2001:200::1": true, "2001:db8::1": false, "2001:db9::1": true, "3ffe:ffff::1": true, "3fff:fff::1": false, "3fff:1000::1": true, "2620:4f:7fff::1": true, "2620:4f:8000::1": false}
	for value, want := range cases {
		if got := IsPublicAddress(netip.MustParseAddr(value)); got != want {
			t.Errorf("%s public=%t want=%t", value, got, want)
		}
	}
	for _, value := range []string{"8.8.8.8", "127.0.0.1", "192.0.0.9"} {
		want := IsPublicAddress(netip.MustParseAddr(value))
		for _, prefix := range []string{"::", "::ffff:"} {
			if IsPublicAddress(netip.MustParseAddr(prefix+value)) != want {
				t.Errorf("normalization changed for %s%s", prefix, value)
			}
		}
	}
}

func TestOutboundCancelledContextKeepsAdmissionErrorPriority(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	client := New(time.Second)
	client.lookup = func(context.Context, string, string) ([]netip.Addr, error) {
		t.Fatal("invalid request reached DNS")
		return nil, errors.New("unexpected lookup")
	}
	cases := []struct {
		url     string
		headers http.Header
		body    any
		want    error
	}{
		{"http://example.invalid", http.Header{"Invalid\nName": {"x"}}, make(chan int), HttpsRequired},
		{"https://example.invalid", http.Header{"Invalid\nName": {"x"}}, make(chan int), RequestFailed},
		{"https://example.invalid", http.Header{"Authorization": {"invalid\nvalue"}}, nil, RequestFailed},
	}
	for _, test := range cases {
		if _, err := client.PostJson(ctx, test.url, test.headers, test.body, time.Second); err != test.want {
			t.Fatalf("admission error displaced: %v want %v", err, test.want)
		}
	}
}

func TestOutboundRequestKeepsCallerHeaderAndTlsOwnership(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Accept") != "" || request.Header.Get("Content-Type") != "application/json" {
			t.Error("explicit empty Accept or default content type changed")
		}
		response.Header().Set("Content-Type", "application/json")
		fmt.Fprint(response, "{}")
	}))
	defer server.Close()
	client := localClient(server)
	configuration := client.tlsConfig
	headers := http.Header{"Accept": nil, "X-Custom": {"retained"}}
	original := headers.Clone()
	if _, err := client.PostJson(context.Background(), server.URL, headers, nil, time.Second); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(headers, original) || client.tlsConfig != configuration || configuration.ServerName != "" {
		t.Fatal("caller headers or TLS configuration mutated")
	}
}
