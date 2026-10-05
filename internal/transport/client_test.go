package transport

import (
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
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) { io.WriteString(response, "ready") }))
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
