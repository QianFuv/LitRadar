// Package transport supplies explicit network policy without ambient proxy inheritance.
package transport

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"golang.org/x/net/proxy"
)

// New creates an HTTP transport whose proxy is exclusively the supplied managed URL.
// SOCKS5 resolves destinations locally; SOCKS5h leaves destination resolution to the proxy.
func New(proxyUrl string) (*http.Transport, error) {
	dialer := &net.Dialer{Timeout: 30 * time.Second, KeepAlive: 30 * time.Second}
	transport := &http.Transport{DialContext: dialer.DialContext, ForceAttemptHTTP2: true, MaxIdleConns: 100,
		IdleConnTimeout: 90 * time.Second, TLSHandshakeTimeout: 10 * time.Second, ExpectContinueTimeout: time.Second}
	if proxyUrl == "" {
		return transport, nil
	}
	location, err := url.Parse(proxyUrl)
	invalid := errors.New("invalid proxy URL")
	if err != nil || location.Hostname() == "" || location.Opaque != "" || location.RawQuery != "" || location.ForceQuery || strings.Contains(proxyUrl, "#") || (location.Path != "" && location.Path != "/") {
		return nil, invalid
	}
	if port := location.Port(); port != "" {
		value, err := strconv.ParseUint(port, 10, 16)
		if err != nil || value == 0 {
			return nil, invalid
		}
	}
	var authentication *proxy.Auth
	if location.User != nil {
		password, exists := location.User.Password()
		if location.User.Username() == "" || !exists || password == "" {
			return nil, invalid
		}
		authentication = &proxy.Auth{User: location.User.Username(), Password: password}
	}
	switch location.Scheme {
	case "http", "https":
		transport.Proxy = http.ProxyURL(location)
	case "socks5", "socks5h":
		port := location.Port()
		if port == "" {
			port = "1080"
		}
		selected, err := proxy.SOCKS5("tcp", net.JoinHostPort(location.Hostname(), port), authentication, dialer)
		if err != nil {
			return nil, invalid
		}
		contextDialer := selected.(proxy.ContextDialer)
		transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
			if location.Scheme == "socks5h" {
				return contextDialer.DialContext(ctx, network, address)
			}
			host, port, err := net.SplitHostPort(address)
			if err != nil {
				return nil, err
			}
			if net.ParseIP(host) != nil {
				return contextDialer.DialContext(ctx, network, address)
			}
			addresses, err := net.DefaultResolver.LookupIPAddr(ctx, host)
			if err != nil {
				return nil, err
			}
			var failures []error
			for _, resolved := range addresses {
				connection, err := contextDialer.DialContext(ctx, network, net.JoinHostPort(resolved.String(), port))
				if err == nil {
					return connection, nil
				}
				failures = append(failures, err)
				if ctx.Err() != nil {
					return nil, ctx.Err()
				}
			}
			if len(failures) == 0 {
				return nil, errors.New("destination has no addresses")
			}
			return nil, errors.Join(failures...)
		}
	default:
		return nil, invalid
	}
	return transport, nil
}
