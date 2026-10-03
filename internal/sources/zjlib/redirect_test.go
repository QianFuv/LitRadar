package zjlib

import (
	"bufio"
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func observeRedirectWire(t *testing.T, request map[string]any) any {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	base := "http://" + listener.Addr().String()
	location, hasLocation := request["location"].(string)
	location = strings.ReplaceAll(location, "{base}", base)
	shouldRedirect := true
	if value, ok := request["redirect"].(bool); ok {
		shouldRedirect = value
	}
	finished := make(chan []string, 1)
	failures := make(chan error, 1)
	go func() {
		captured := []string{}
		defer func() { finished <- captured }()
		for {
			connection, err := listener.Accept()
			if err != nil {
				return
			}
			connection.SetDeadline(time.Now().Add(5 * time.Second))
			reader := bufio.NewReader(connection)
			line, err := reader.ReadString('\n')
			if err != nil {
				connection.Close()
				failures <- err
				return
			}
			parts := strings.Fields(line)
			if len(parts) < 2 {
				connection.Close()
				failures <- fmt.Errorf("missing request target")
				return
			}
			captured = append(captured, parts[1])
			for {
				line, err = reader.ReadString('\n')
				if err != nil {
					connection.Close()
					failures <- err
					return
				}
				if line == "\r\n" {
					break
				}
			}
			body := "%PDF"
			if !shouldRedirect {
				body = `{"data":{"uuid":"qr","qrCode":"image","status":"WAITING_SCAN"}}`
			}
			status := 200
			headers := ""
			if len(captured) == 1 {
				status = 302
				if hasLocation {
					headers = "Location: " + location + "\r\n"
				}
			}
			fmt.Fprintf(connection, "HTTP/1.1 %d Response\r\nConnection: close\r\nContent-Type: application/pdf\r\nContent-Length: %d\r\n%s\r\n%s", status, len(body), headers, body)
			connection.Close()
		}
	}()
	allowed := endpoints{bases: [4]string{base + "/www", base + "/share", base + "/login", base + "/proxy"}, entry: base + "/share/entry", referer: base + "/proxy/kns55/"}
	live, err := newLiveTransport(LiveConfig{TimeoutSeconds: 3, MaximumDocumentBytes: 1024}, transport.Proxy{}, time.Time{}, allowed)
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	var value any
	if shouldRedirect {
		var pdf DownloadedPdf
		pdf, err = live.DownloadPdf(context.Background(), base+"/proxy/start", nil, nil)
		value = map[string]any{"final": strings.TrimPrefix(pdf.FinalUrl, base), "content": string(pdf.Content)}
	} else {
		value, err = live.StartQrLogin(context.Background())
	}
	listener.Close()
	captured := <-finished
	select {
	case failure := <-failures:
		t.Fatal(failure)
	default:
	}
	return map[string]any{"requests": captured, "result": observedOutcome(value, err)}
}

func TestLiveExplicitHttpProxyUsesAbsoluteRequestTarget(t *testing.T) {
	proxyServer := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.RequestURI != "http://upstream.invalid/proxy/download.aspx?bad=%GG" || request.Host != "upstream.invalid" {
			t.Errorf("proxy request target/host mismatch: %s %s", request.RequestURI, request.Host)
		}
		writer.Header().Set("Content-Type", "application/pdf")
		fmt.Fprint(writer, "%PDF")
	}))
	defer proxyServer.Close()
	selection, err := transport.ExplicitProxy(proxyServer.URL)
	if err != nil {
		t.Fatal(err)
	}
	allowed := endpoints{bases: [4]string{"http://upstream.invalid/www", "http://upstream.invalid/share", "http://upstream.invalid/login", "http://upstream.invalid/proxy"}}
	live, err := newLiveTransport(DefaultLiveConfig(), selection, time.Time{}, allowed)
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	if _, err = live.DownloadPdf(context.Background(), "http://upstream.invalid/proxy/download.aspx?bad=%GG", nil, nil); err != nil {
		t.Fatal(err)
	}
}
func TestLiveTlsRequestRetainsAuthority(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.RequestURI != "/proxy/download.aspx" || request.Host == "request.invalid" {
			t.Errorf("TLS request authority mismatch: %s %s", request.RequestURI, request.Host)
		}
		writer.Header().Set("Content-Type", "application/pdf")
		fmt.Fprint(writer, "%PDF")
	}))
	defer server.Close()
	allowed := endpoints{bases: [4]string{server.URL + "/www", server.URL + "/share", server.URL + "/login", server.URL + "/proxy"}}
	live, err := newLiveTransport(DefaultLiveConfig(), transport.Proxy{}, time.Time{}, allowed)
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	live.state.redirect.Transport.(*transport.ClientTransport).TLSClientConfig = &tls.Config{RootCAs: roots}
	if _, err = live.DownloadPdf(context.Background(), server.URL+"/proxy/download.aspx", nil, nil); err != nil {
		t.Fatal(err)
	}
}
