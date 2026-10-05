package zjlib

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"

	"net/http"
	"net/http/httptest"

	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

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
