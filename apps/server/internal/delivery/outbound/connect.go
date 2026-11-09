package outbound

import (
	"context"
	"crypto/tls"
	"net"
	"net/netip"
	"time"
)

func (client *Client) connect(ctx, requestContext context.Context, network, address string, tlsConfig *tls.Config) (net.Conn, error) {
	ctx, cancel := context.WithTimeout(ctx, client.connectTimeout)
	defer cancel()
	stop := context.AfterFunc(requestContext, cancel)
	defer stop()
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, err
	}
	addresses, err := client.resolve(ctx, host)
	if err != nil {
		return nil, err
	}
	socket, err := dialResolved(ctx, network, port, addresses, (&net.Dialer{}).DialContext)
	if err != nil {
		return nil, err
	}
	if tlsConfig == nil {
		return socket, nil
	}
	configuration := tlsConfig.Clone()
	if configuration.ServerName == "" {
		configuration.ServerName = host
	}
	secured := tls.Client(socket, configuration)
	if err := secured.HandshakeContext(ctx); err != nil {
		socket.Close()
		return nil, err
	}
	return secured, nil
}

// DialResolved connects only to the caller's validated addresses, racing address families after 300 ms.
func DialResolved(ctx context.Context, network, port string, addresses []netip.Addr, dial func(context.Context, string, string) (net.Conn, error)) (net.Conn, error) {
	if len(addresses) == 0 {
		return nil, &net.DNSError{Err: "no addresses"}
	}
	return dialResolved(ctx, network, port, addresses, dial)
}

func dialResolved(ctx context.Context, network, port string, addresses []netip.Addr, dial func(context.Context, string, string) (net.Conn, error)) (net.Conn, error) {
	preferred, fallback := []netip.Addr{}, []netip.Addr{}
	for _, address := range addresses {
		if address.Is4() == addresses[0].Is4() {
			preferred = append(preferred, address)
		} else {
			fallback = append(fallback, address)
		}
	}
	serial := func(ctx context.Context, group []netip.Addr) (net.Conn, error) {
		var firstError error
		for _, address := range group {
			socket, err := dial(ctx, network, net.JoinHostPort(address.String(), port))
			if err == nil {
				return socket, nil
			}
			if firstError == nil {
				firstError = err
			}
			if ctx.Err() != nil {
				break
			}
		}
		return nil, firstError
	}
	if len(fallback) == 0 {
		return serial(ctx, preferred)
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	type result struct {
		socket net.Conn
		err    error
	}
	results := make(chan result)
	start := func(group []netip.Addr) {
		go func() {
			socket, err := serial(ctx, group)
			select {
			case results <- result{socket, err}:
			case <-ctx.Done():
				if socket != nil {
					socket.Close()
				}
			}
		}()
	}
	start(preferred)
	timer := time.NewTimer(300 * time.Millisecond)
	defer timer.Stop()
	pending, hasFallback := 1, false
	var lastError error
	for pending > 0 {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-timer.C:
			start(fallback)
			pending++
			hasFallback = true
		case completed := <-results:
			pending--
			if completed.err == nil {
				return completed.socket, nil
			}
			lastError = completed.err
			if !hasFallback {
				timer.Stop()
				start(fallback)
				pending++
				hasFallback = true
			}
		}
	}
	return nil, lastError
}
