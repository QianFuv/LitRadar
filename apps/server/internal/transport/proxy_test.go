package transport

import (
	"bufio"
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestProxyCredentialDecodingOnHttpWire(t *testing.T) {
	for encoded, decoded := range map[string]string{"u%25zz": "u%zz", "u%FF": "u\ufffd", "u%F1%80%80": "u\ufffd", "u+name": "u+name", "u%40name": "u@name"} {
		t.Run(encoded, func(t *testing.T) {
			capture := make(chan string, 1)
			server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				capture <- request.Header.Get("Proxy-Authorization")
				if _, err := io.WriteString(writer, "ok"); err != nil {
					t.Errorf("io.WriteString: %v", err)
				}
			}))
			defer server.Close()
			proxy, err := ExplicitProxy(strings.Replace(server.URL, "://", "://"+encoded+":p@", 1))
			if err != nil {
				t.Fatal(err)
			}
			wire, err := proxy.ClientTransport()
			if err != nil {
				t.Fatal(err)
			}
			defer wire.CloseIdleConnections()
			client := http.Client{Transport: wire, Timeout: time.Second}
			response, err := client.Get("http://unresolvable.invalid/article")
			if err != nil {
				t.Fatal(err)
			}
			if err := response.Body.Close(); err != nil {
				t.Errorf("response.Body.Close: %v", err)
			}
			want := "Basic " + base64.StdEncoding.EncodeToString([]byte(decoded+":p"))
			if actual := <-capture; actual != want {
				t.Fatalf("%q want %q", actual, want)
			}
		})
	}
}

func TestProxyCredentialDecodingOnSocksWire(t *testing.T) {
	for _, scheme := range []string{"socks5", "socks5h"} {
		t.Run(scheme, func(t *testing.T) {
			listener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := listener.Close(); err != nil {
					t.Errorf("listener.Close: %v", err)
				}
			}()
			capture := make(chan []string, 1)
			failures := make(chan error, 1)
			go func() {
				connection, err := listener.Accept()
				if err != nil {
					failures <- err
					return
				}
				defer func() {
					if err := connection.Close(); err != nil {
						t.Errorf("connection.Close: %v", err)
					}
				}()
				if err := connection.SetDeadline(time.Now().Add(3 * time.Second)); err != nil {
					t.Errorf("connection.SetDeadline: %v", err)
				}
				read := func(count int) ([]byte, error) {
					result := make([]byte, count)
					_, err := io.ReadFull(connection, result)
					return result, err
				}
				greeting, err := read(2)
				if err != nil {
					failures <- err
					return
				}
				if _, err = read(int(greeting[1])); err != nil {
					failures <- err
					return
				}
				if _, err := connection.Write([]byte{5, 2}); err != nil {
					t.Errorf("connection.Write: %v", err)
				}
				authentication, err := read(2)
				if err != nil {
					failures <- err
					return
				}
				username, err := read(int(authentication[1]))
				if err != nil {
					failures <- err
					return
				}
				passwordLength, err := read(1)
				if err != nil {
					failures <- err
					return
				}
				password, err := read(int(passwordLength[0]))
				if err != nil {
					failures <- err
					return
				}
				capture <- []string{string(username), string(password)}
				if _, err := connection.Write([]byte{1, 0}); err != nil {
					t.Errorf("connection.Write: %v", err)
				}
				command, err := read(4)
				if err != nil {
					failures <- err
					return
				}
				count := 4
				switch command[3] {
				case 3:
					length, err := read(1)
					if err != nil {
						failures <- err
						return
					}
					count = int(length[0])
				case 4:
					count = 16
				}
				if _, err = read(count + 2); err != nil {
					failures <- err
					return
				}
				if _, err := connection.Write([]byte{5, 0, 0, 1, 127, 0, 0, 1, 0, 80}); err != nil {
					t.Errorf("connection.Write: %v", err)
				}
				request, err := http.ReadRequest(bufio.NewReader(connection))
				if err != nil {
					failures <- err
					return
				}
				if err := request.Body.Close(); err != nil {
					t.Errorf("request.Body.Close: %v", err)
				}
				_, err = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
				failures <- err
			}()
			proxy, err := ExplicitProxy(scheme + "://u%FF:p%25zz@" + listener.Addr().String())
			if err != nil {
				t.Fatal(err)
			}
			wire, err := proxy.ClientTransport()
			if err != nil {
				t.Fatal(err)
			}
			defer wire.CloseIdleConnections()
			client := http.Client{Transport: wire, Timeout: 3 * time.Second}
			response, err := client.Get("http://127.0.0.1:80/article")
			if err != nil {
				t.Fatal(err)
			}
			if err := response.Body.Close(); err != nil {
				t.Errorf("response.Body.Close: %v", err)
			}
			if err := <-failures; err != nil {
				t.Fatal(err)
			}
			credentials := <-capture
			if credentials[0] != "u\ufffd" || credentials[1] != "p%zz" {
				t.Fatalf("unexpected decoded credentials %q", credentials)
			}
		})
	}
}

// TestProxyFormattingAndExplicitFailure proves redaction and explicit versus direct proxy decisions.
func TestProxyFormattingAndExplicitFailure(t *testing.T) {
	proxy, err := ExplicitProxy("http://private-user:private-password@127.0.0.1:1")
	if err != nil {
		t.Fatal(err)
	}
	assertProxyRedaction(t, proxy)
	wire, err := proxy.ClientTransport()
	if err != nil {
		t.Fatal(err)
	}
	defer wire.CloseIdleConnections()
	if wire.Proxy == nil {
		t.Fatal("explicit decision became direct")
	}
	request, err := http.NewRequestWithContext(context.Background(), http.MethodGet, "http://unresolvable.invalid/", nil)
	if err != nil {
		t.Fatal(err)
	}
	if selected, err := wire.Proxy(request); err != nil || selected == nil {
		t.Fatal("explicit decision lost", err)
	}
	direct, err := (Proxy{}).Transport()
	if err != nil {
		t.Fatal(err)
	}
	defer direct.CloseIdleConnections()
	if direct.Proxy != nil {
		t.Fatal("direct decision inherits environment")
	}
}

// assertProxyRedaction checks logging and all implicit formatting for value and pointer forms.
func assertProxyRedaction(t *testing.T, proxy Proxy) {
	t.Helper()
	for _, value := range []any{proxy, &proxy} {
		var output strings.Builder
		logger := slog.New(slog.NewJSONHandler(&output, nil))
		logger.Info("proxy", slog.Any("value", value))
		output.WriteString(fmt.Sprintf("%v %+v %#v", value, value, value))
		if strings.Contains(output.String(), "private-") {
			t.Fatal("proxy credentials leaked")
		}
	}
}

// TestProxyRejectsMalformedCredentialEscapes prevents inconsistent URL and wire credentials.
func TestProxyRejectsMalformedCredentialEscapes(t *testing.T) {
	for _, value := range []string{"http://u%zz:p@proxy.test:80", "socks5://u:p%zz@proxy.test:1080"} {
		if _, err := ExplicitProxy(value); err != ErrProxyUrl {
			t.Fatalf("malformed proxy credential accepted: %v", err)
		}
	}
}
