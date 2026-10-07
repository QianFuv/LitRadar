package api

import (
	"net/http"
	"net/netip"
	"testing"
)

func TestClientSourceTrustsOnlyConfiguredProxyChain(t *testing.T) {
	trusted := []netip.Prefix{netip.MustParsePrefix("10.0.0.0/8")}
	for _, scenario := range []struct {
		name, peer     string
		headers        http.Header
		address, class string
	}{
		{"missing peer", "", http.Header{"Forwarded": {"for=192.0.2.1"}}, "::", "missing_peer"},
		{"direct", "192.0.2.2", nil, "192.0.2.2", "direct"},
		{"untrusted spoof", "192.0.2.2", http.Header{"X-Forwarded-For": {"192.0.2.1"}}, "192.0.2.2", "untrusted_forwarding_header"},
		{"trusted no header", "10.0.0.1", nil, "10.0.0.1", "trusted_proxy_without_header"},
		{"rightmost untrusted", "10.0.0.1", http.Header{"X-Forwarded-For": {"192.0.2.1, 198.51.100.1, 10.0.0.2"}}, "198.51.100.1", "trusted_forwarding_chain"},
		{"forwarded precedence", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3"}, "X-Forwarded-For": {"192.0.2.4"}}, "192.0.2.3", "trusted_forwarding_chain"},
		{"invalid does not fall back", "10.0.0.1", http.Header{"Forwarded": {"for=unknown"}, "X-Forwarded-For": {"192.0.2.4"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
		{"empty forwarded does not fall back", "10.0.0.1", http.Header{"Forwarded": {""}, "X-Forwarded-For": {"192.0.2.4"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
		{"invalid later header invalidates chain", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3", "for=unknown"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
		{"all header values", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3;proto=https", "for=10.0.0.2"}}, "192.0.2.3", "trusted_forwarding_chain"},
		{"duplicate for", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3;FOR=192.0.2.4"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
		{"bad unknown parameter", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3;proto"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
		{"mapped peer", "::ffff:10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3"}}, "192.0.2.3", "trusted_forwarding_chain"},
		{"tab whitespace", "10.0.0.1", http.Header{"Forwarded": {"for=\t192.0.2.3\t"}}, "192.0.2.3", "trusted_forwarding_chain"},
		{"nonascii header", "10.0.0.1", http.Header{"Forwarded": {"for=192.0.2.3;extra=文"}}, "10.0.0.1", "trusted_proxy_invalid_header"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			peer, _ := netip.ParseAddr(scenario.peer)
			result := resolveAuthClientSource(peer, scenario.headers, trusted)
			if result.address.String() != scenario.address || result.class != scenario.class {
				t.Fatalf("source: %+v", result)
			}
		})
	}
	mapped := []netip.Prefix{netip.MustParsePrefix("::ffff:10.0.0.0/104")}
	if resolveAuthClientSource(netip.MustParseAddr("::ffff:10.0.0.1"), nil, mapped).class != "direct" {
		t.Fatal("CIDR address family was rewritten")
	}
}

func TestForwardedNodesPreserveLegacyGrammar(t *testing.T) {
	for _, raw := range []string{"192.0.2.1", "192.0.2.1:80", "[192.0.2.1]", "\"192.0.2.1\"", "::ffff:192.0.2.1", "[::ffff:192.0.2.1]:80"} {
		if address := parseForwardedNode(raw); address.String() != "192.0.2.1" {
			t.Fatalf("%q: %s", raw, address)
		}
	}
	for _, raw := range []string{"unknown", "host.invalid", "_hidden", "\" 192.0.2.1 \"", "\"192.0.2.1\\\"", "[fe80::1%eth0]", "[fe80::1%1]:80", "192.0.2.1:65536", "192.0.2.1:"} {
		if address := parseForwardedNode(raw); address.IsValid() {
			t.Fatalf("accepted invalid %q: %s", raw, address)
		}
	}
}
