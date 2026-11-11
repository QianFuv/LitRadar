package transport

import (
	"compress/gzip"
	"io"
	"net"
	"net/http"
	"net/http/httptest"

	"sync"
	"testing"
	"time"
)

type delayedHandshakeListener struct {
	net.Listener
	delay time.Duration
}
type delayedHandshakeConnection struct {
	net.Conn
	delay time.Duration
	once  sync.Once
}

func (listener delayedHandshakeListener) Accept() (net.Conn, error) {
	connection, err := listener.Listener.Accept()
	if err != nil {
		return nil, err
	}
	return &delayedHandshakeConnection{Conn: connection, delay: listener.delay}, nil
}
func (connection *delayedHandshakeConnection) Read(body []byte) (int, error) {
	connection.once.Do(func() { time.Sleep(connection.delay) })
	return connection.Conn.Read(body)
}

func TestSourceTlsHandshakeUsesRequestBudget(t *testing.T) {
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if _, err := io.WriteString(response, "ready"); err != nil {
			t.Errorf("io.WriteString: %v", err)
		}
	}))
	server.Listener = delayedHandshakeListener{server.Listener, 10500 * time.Millisecond}
	server.StartTLS()
	defer server.Close()
	wire, err := (Proxy{}).ClientTransport()
	if err != nil {
		t.Fatal(err)
	}
	defer wire.CloseIdleConnections()
	wire.TLSClientConfig = server.Client().Transport.(*http.Transport).TLSClientConfig.Clone()
	client := http.Client{Transport: wire, Timeout: 20 * time.Second}
	response, err := client.Get(server.URL)
	if err != nil {
		t.Fatalf("handshake stopped before caller budget: %v", err)
	}
	body, err := BoundedText(response, 64)
	if err != nil || body != "ready" {
		t.Fatalf("%q %v", body, err)
	}
}

// TestSourceGzipUsesStandardMultistreamDecoding covers the decoded body and response metadata.
func TestSourceGzipUsesStandardMultistreamDecoding(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Accept-Encoding") != "gzip" {
			t.Error("gzip negotiation missing")
		}
		writer.Header().Set("Content-Encoding", "GZIP")
		for _, text := range []string{"first", "second"} {
			compressed := gzip.NewWriter(writer)
			if _, err := io.WriteString(compressed, text); err != nil {
				t.Error(err)
			}
			if err := compressed.Close(); err != nil {
				t.Error(err)
			}
		}
	}))
	t.Cleanup(server.Close)
	wire, err := (Proxy{}).ClientTransport()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(wire.CloseIdleConnections)
	client := http.Client{Transport: wire}
	response, err := client.Get(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	body, err := BoundedText(response, 64)
	if err != nil || body != "firstsecond" || !response.Uncompressed || response.Header.Get("Content-Encoding") != "" {
		t.Fatalf("standard gzip decoding failed: %q %v", body, err)
	}
}
