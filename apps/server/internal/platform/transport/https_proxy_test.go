package transport

import (
	"crypto/tls"
	"crypto/x509"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestHttpsProxyConnectRetainsTlsVerification(t *testing.T) {
	origin := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Proxy-Authorization") != "" {
			t.Error("proxy authentication leaked into tunnel")
		}
		io.WriteString(writer, "tunneled")
	}))
	defer origin.Close()
	var calls atomic.Int32
	proxyServer := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Method != "CONNECT" || request.Host != strings.TrimPrefix(origin.URL, "https://") {
			t.Error("unexpected CONNECT destination")
			writer.WriteHeader(400)
			return
		}
		if request.Header.Get("Proxy-Authorization") != "Basic Zml4dHVyZTpwYXNzd29yZA==" {
			t.Error("missing tunnel authentication")
			writer.WriteHeader(407)
			return
		}
		upstream, err := net.DialTimeout("tcp", request.Host, time.Second)
		if err != nil {
			t.Error(err)
			writer.WriteHeader(502)
			return
		}
		defer upstream.Close()
		connection, buffer, err := writer.(http.Hijacker).Hijack()
		if err != nil {
			t.Error(err)
			return
		}
		defer connection.Close()
		connection.SetDeadline(time.Now().Add(5 * time.Second))
		upstream.SetDeadline(time.Now().Add(5 * time.Second))
		buffer.WriteString("HTTP/1.1 200 Connection Established\r\n\r\n")
		buffer.Flush()
		calls.Add(1)
		var copying sync.WaitGroup
		copying.Go(func() { io.Copy(upstream, buffer); upstream.Close() })
		io.Copy(connection, upstream)
		connection.Close()
		copying.Wait()
	}))
	defer proxyServer.Close()
	proxyUrl := strings.Replace(proxyServer.URL, "https://", "https://fixture:password@", 1)
	selected, err := New(proxyUrl)
	if err != nil {
		t.Fatal(err)
	}
	defer selected.CloseIdleConnections()
	roots := x509.NewCertPool()
	roots.AddCert(proxyServer.Certificate())
	roots.AddCert(origin.Certificate())
	selected.TLSClientConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	client := &http.Client{Transport: selected, Timeout: 3 * time.Second}
	response, err := client.Get(origin.URL)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || string(body) != "tunneled" || calls.Load() != 1 {
		t.Fatalf("CONNECT result %q %v calls=%d", body, err, calls.Load())
	}
	selected.CloseIdleConnections()
	untrusted, _ := New(proxyUrl)
	defer untrusted.CloseIdleConnections()
	untrusted.TLSClientConfig = &tls.Config{RootCAs: x509.NewCertPool(), MinVersion: tls.VersionTLS12}
	if response, err := (&http.Client{Transport: untrusted, Timeout: 3 * time.Second}).Get(origin.URL); err == nil {
		response.Body.Close()
		t.Fatal("untrusted proxy TLS accepted")
	}
	if calls.Load() != 1 {
		t.Fatal("untrusted proxy reached CONNECT")
	}
}
