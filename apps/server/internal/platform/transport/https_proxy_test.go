package transport

import (
	"crypto/tls"
	"crypto/x509"
	"errors"
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
		if _, err := io.WriteString(writer, "tunneled"); err != nil {
			t.Errorf("io.WriteString: %v", err)
		}
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
		defer func() {
			if err := upstream.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Errorf("upstream.Close: %v", err)
			}
		}()
		connection, buffer, err := writer.(http.Hijacker).Hijack()
		if err != nil {
			t.Error(err)
			return
		}
		defer func() {
			if err := connection.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Errorf("connection.Close: %v", err)
			}
		}()
		if err := connection.SetDeadline(time.Now().Add(5 * time.Second)); err != nil {
			t.Errorf("connection.SetDeadline: %v", err)
		}
		if err := upstream.SetDeadline(time.Now().Add(5 * time.Second)); err != nil {
			t.Errorf("upstream.SetDeadline: %v", err)
		}
		if _, err := buffer.WriteString("HTTP/1.1 200 Connection Established\r\n\r\n"); err != nil {
			t.Errorf("buffer.WriteString: %v", err)
		}
		if err := buffer.Flush(); err != nil {
			t.Errorf("buffer.Flush: %v", err)
		}
		calls.Add(1)
		var copying sync.WaitGroup
		copying.Go(func() {
			if _, err := io.Copy(upstream, buffer); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Errorf("io.Copy: %v", err)
			}
			if err := upstream.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Errorf("upstream.Close: %v", err)
			}
		})
		if _, err := io.Copy(connection, upstream); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Errorf("io.Copy: %v", err)
		}
		if err := connection.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Errorf("connection.Close: %v", err)
		}
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
	if err := response.Body.Close(); err != nil {
		t.Fatalf("response.Body.Close: %v", err)
	}
	if err != nil || string(body) != "tunneled" || calls.Load() != 1 {
		t.Fatalf("CONNECT result %q %v calls=%d", body, err, calls.Load())
	}
	selected.CloseIdleConnections()
	untrusted, _ := New(proxyUrl)
	defer untrusted.CloseIdleConnections()
	untrusted.TLSClientConfig = &tls.Config{RootCAs: x509.NewCertPool(), MinVersion: tls.VersionTLS12}
	if response, err := (&http.Client{Transport: untrusted, Timeout: 3 * time.Second}).Get(origin.URL); err == nil {
		if err := response.Body.Close(); err != nil {
			t.Fatalf("response.Body.Close: %v", err)
		}
		t.Fatal("untrusted proxy TLS accepted")
	}
	if calls.Load() != 1 {
		t.Fatal("untrusted proxy reached CONNECT")
	}
}
