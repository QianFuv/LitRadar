package transport

import (
	"bufio"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestFrozenReqwestWire(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/wire-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures struct {
		Observations []struct {
			Id       string
			Response []int
			Limit    int64
			Output   struct {
				Body         struct{ Text, Error string }
				Accept, Gzip bool
			}
		}
	}
	if err := json.Unmarshal(body, &fixtures); err != nil {
		t.Fatal(err)
	}
	for _, fixture := range fixtures.Observations {
		t.Run(fixture.Id, func(t *testing.T) {
			listener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			defer listener.Close()
			captured := make(chan *http.Request, 1)
			failures := make(chan error, 1)
			go func() {
				connection, err := listener.Accept()
				if err != nil {
					failures <- err
					return
				}
				defer connection.Close()
				connection.SetDeadline(time.Now().Add(3 * time.Second))
				request, err := http.ReadRequest(bufio.NewReader(connection))
				if err != nil {
					failures <- err
					return
				}
				captured <- request
				response := make([]byte, len(fixture.Response))
				for index, value := range fixture.Response {
					response[index] = byte(value)
				}
				_, err = connection.Write(response)
				failures <- err
			}()
			wire, err := (Proxy{}).ClientTransport()
			if err != nil {
				t.Fatal(err)
			}
			defer wire.CloseIdleConnections()
			client := http.Client{Transport: wire, Timeout: 3 * time.Second}
			response, err := client.Get("http://" + listener.Addr().String() + "/")
			if err != nil {
				t.Fatal(err)
			}
			text, err := BoundedText(response, fixture.Limit)
			if fixture.Output.Body.Error != "" {
				if err == nil || err.Error() != fixture.Output.Body.Error {
					t.Fatalf("body error %v want %s", err, fixture.Output.Body.Error)
				}
			} else if err != nil || text != fixture.Output.Body.Text {
				t.Fatalf("body %q %v want %q", text, err, fixture.Output.Body.Text)
			}
			request := <-captured
			if (request.Header.Get("Accept") == "*/*") != fixture.Output.Accept || (request.Header.Get("Accept-Encoding") == "gzip") != fixture.Output.Gzip {
				t.Fatalf("headers %#v", request.Header)
			}
			if err := <-failures; err != nil && !strings.Contains(err.Error(), "closed") && err != io.ErrClosedPipe {
				t.Fatal(err)
			}
		})
	}
}

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
