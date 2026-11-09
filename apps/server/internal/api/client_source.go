package api

import (
	"net/http"
	"net/netip"
	"strings"
)

func resolveAuthClientSource(peer netip.Addr, headers http.Header, trusted []netip.Prefix) authClientSource {
	if !peer.IsValid() {
		return authClientSource{netip.IPv6Unspecified(), "missing_peer"}
	}
	peer = peer.Unmap()
	if !isTrustedProxy(peer, trusted) {
		class := "direct"
		if headers.Values("Forwarded") != nil || headers.Values("X-Forwarded-For") != nil {
			class = "untrusted_forwarding_header"
		}
		return authClientSource{peer, class}
	}
	chain, class := parseForwardingChain(headers)
	if class != "trusted_forwarding_chain" {
		return authClientSource{peer, class}
	}
	client := peer
	for index := len(chain) - 1; index >= 0; index-- {
		if !isTrustedProxy(client, trusted) {
			break
		}
		client = chain[index].Unmap()
	}
	return authClientSource{client, class}
}

func isTrustedProxy(address netip.Addr, trusted []netip.Prefix) bool {
	for _, network := range trusted {
		if network.Contains(address) {
			return true
		}
	}
	return false
}

// parseForwardingChain gives Forwarded precedence and invalidates the entire malformed chain.
func parseForwardingChain(headers http.Header) ([]netip.Addr, string) {
	values := headers.Values("Forwarded")
	isForwarded := len(values) != 0
	if !isForwarded {
		values = headers.Values("X-Forwarded-For")
	}
	if len(values) == 0 {
		return nil, "trusted_proxy_without_header"
	}
	chain := []netip.Addr{}
	for _, value := range values {
		if !validForwardingHeader(value) {
			return nil, "trusted_proxy_invalid_header"
		}
		for _, element := range strings.Split(value, ",") {
			address := forwardingElement(element, isForwarded)
			if !address.IsValid() {
				return nil, "trusted_proxy_invalid_header"
			}
			chain = append(chain, address)
		}
	}
	return chain, "trusted_forwarding_chain"
}

// validForwardingHeader permits tabs but rejects other controls and non-ASCII bytes.
func validForwardingHeader(value string) bool {
	for _, character := range []byte(value) {
		if (character < 32 && character != '\t') || character >= 127 {
			return false
		}
	}
	return true
}

// forwardingElement requires exactly one valid for parameter when using Forwarded syntax.
func forwardingElement(element string, isForwarded bool) netip.Addr {
	if !isForwarded {
		return parseForwardedNode(strings.TrimSpace(element))
	}
	var address netip.Addr
	for _, parameter := range strings.Split(element, ";") {
		name, raw, hasEquals := strings.Cut(strings.TrimSpace(parameter), "=")
		if !hasEquals {
			return netip.Addr{}
		}
		if asciiLower(strings.TrimSpace(name)) != "for" {
			continue
		}
		if address.IsValid() {
			return netip.Addr{}
		}
		address = parseForwardedNode(strings.TrimSpace(raw))
		if !address.IsValid() {
			return netip.Addr{}
		}
	}
	return address
}

// parseForwardedNode retains address, port and legacy bracketed-address forms without zones.
func parseForwardedNode(value string) netip.Addr {
	unquoted, isValid := unquoteForwardedNode(value)
	if !isValid {
		return netip.Addr{}
	}
	value = unquoted
	if address, err := netip.ParseAddr(value); err == nil && address.Zone() == "" {
		return address.Unmap()
	}
	if endpoint, err := netip.ParseAddrPort(value); err == nil && endpoint.Addr().Zone() == "" {
		return endpoint.Addr().Unmap()
	}
	if strings.HasPrefix(value, "[") && strings.HasSuffix(value, "]") {
		if address, err := netip.ParseAddr(value[1 : len(value)-1]); err == nil && address.Zone() == "" {
			return address.Unmap()
		}
	}
	return netip.Addr{}
}

// unquoteForwardedNode retains paired quotes and rejects escapes or unmatched quotes.
func unquoteForwardedNode(value string) (string, bool) {
	if len(value) >= 2 && strings.HasPrefix(value, "\"") && strings.HasSuffix(value, "\"") {
		value = value[1 : len(value)-1]
		if strings.ContainsAny(value, "\"\\") {
			return "", false
		}
	} else if strings.Contains(value, "\"") {
		return "", false
	}
	return value, true
}
