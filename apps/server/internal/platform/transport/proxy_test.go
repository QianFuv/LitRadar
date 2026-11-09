package transport

import (
	"bufio"
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestExplicitHttpProxyAndNoDirectFallback(t *testing.T) {
	var originCalls atomic.Int32
	origin := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		originCalls.Add(1)
		if request.Header.Get("Proxy-Authorization") != "" {
			t.Error("proxy credential reached origin")
		}
		io.WriteString(writer, "origin")
	}))
	defer origin.Close()
	var proxyCalls atomic.Int32
	forward, _ := New("")
	defer forward.CloseIdleConnections()
	proxyServer := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		proxyCalls.Add(1)
		if request.Header.Get("Proxy-Authorization") != "Basic "+base64.StdEncoding.EncodeToString([]byte("fixture:password")) {
			t.Error("missing proxy authentication")
		}
		if !request.URL.IsAbs() {
			t.Error("proxy request was not absolute-form")
		}
		request.RequestURI = ""
		request.Header.Del("Proxy-Authorization")
		response, err := forward.RoundTrip(request)
		if err != nil {
			t.Error(err)
			writer.WriteHeader(502)
			return
		}
		defer response.Body.Close()
		io.Copy(writer, response.Body)
	}))
	defer proxyServer.Close()
	selected, err := New(strings.Replace(proxyServer.URL, "http://", "http://fixture:password@", 1))
	if err != nil {
		t.Fatal(err)
	}
	defer selected.CloseIdleConnections()
	client := &http.Client{Transport: selected, Timeout: time.Second}
	response, err := client.Get(origin.URL)
	if err != nil {
		t.Fatal(err)
	}
	data, _ := io.ReadAll(response.Body)
	response.Body.Close()
	if string(data) != "origin" || proxyCalls.Load() != 1 || originCalls.Load() != 1 {
		t.Fatal("explicit proxy path was not exercised")
	}
	proxyServer.Close()
	if response, err := client.Get(origin.URL); err == nil {
		response.Body.Close()
		t.Fatal("failed proxy fell back to direct")
	}
	if originCalls.Load() != 1 {
		t.Fatal("origin reached after proxy failure")
	}
}

func TestSocksDnsAndAuthentication(t *testing.T) {
	for _, scheme := range []string{"socks5", "socks5h"} {
		t.Run(scheme, func(t *testing.T) {
			listener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			defer listener.Close()
			done := make(chan error, 1)
			go func() { done <- serveSocksFixture(listener, scheme) }()
			selected, err := New(scheme + "://fixture:password@" + listener.Addr().String())
			if err != nil {
				t.Fatal(err)
			}
			defer selected.CloseIdleConnections()
			host := "localhost"
			if scheme == "socks5h" {
				host = "only-proxy-knows.invalid"
			}
			response, err := (&http.Client{Transport: selected, Timeout: 3 * time.Second}).Get("http://" + host + ":8080/")
			if err != nil {
				t.Fatal(err)
			}
			body, _ := io.ReadAll(response.Body)
			response.Body.Close()
			if string(body) != "fixture" {
				t.Fatalf("SOCKS origin %q", body)
			}
			if err := <-done; err != nil {
				t.Fatal(err)
			}
		})
	}
}

// serveSocksFixture owns one connection and its deadline throughout the protocol exchange.
func serveSocksFixture(listener net.Listener, scheme string) error {
	connection, err := listener.Accept()
	if err != nil {
		return err
	}
	defer connection.Close()
	connection.SetDeadline(time.Now().Add(3 * time.Second))
	reader := bufio.NewReader(connection)
	header := make([]byte, 2)
	if err := socksFixtureGreeting(reader, connection, header); err != nil {
		return err
	}
	if err := socksFixtureAuthentication(reader, connection, header); err != nil {
		return err
	}
	if err := socksFixtureDestination(reader, connection, header, scheme); err != nil {
		return err
	}
	message, err := http.ReadRequest(reader)
	if err != nil {
		return err
	}
	message.Body.Close()
	_, err = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture")
	return err
}

func TestAmbientProxyIsDisabledWithPositiveControl(t *testing.T) {
	if os.Getenv("LITRADAR_PROXY_CHILD") == "1" {
		selected, _ := New("")
		selected.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, network, os.Getenv("LITRADAR_PROXY_ORIGIN"))
		}
		response, err := (&http.Client{Transport: selected, Timeout: time.Second}).Get("http://origin.fixture.invalid/")
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		selected.CloseIdleConnections()
		response, err = (&http.Client{Timeout: time.Second}).Get("http://origin.fixture.invalid/")
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		return
	}
	var originCalls, proxyCalls atomic.Int32
	origin := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) { originCalls.Add(1) }))
	defer origin.Close()
	proxyServer := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) { proxyCalls.Add(1) }))
	defer proxyServer.Close()
	executable, _ := os.Executable()
	command := exec.Command(executable, "-test.run=^TestAmbientProxyIsDisabledWithPositiveControl$")
	for _, entry := range os.Environ() {
		name := strings.ToUpper(strings.SplitN(entry, "=", 2)[0])
		if !strings.Contains(name, "PROXY") && name != "REQUEST_METHOD" {
			command.Env = append(command.Env, entry)
		}
	}
	command.Env = append(command.Env, "HTTP_PROXY="+proxyServer.URL, "LITRADAR_PROXY_CHILD=1", "LITRADAR_PROXY_ORIGIN="+strings.TrimPrefix(origin.URL, "http://"))
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("proxy child %v %s", err, output)
	}
	if originCalls.Load() != 1 || proxyCalls.Load() != 1 {
		t.Fatalf("direct=%d ambient-control=%d", originCalls.Load(), proxyCalls.Load())
	}
}

// socksFixtureGreeting consumes the offered methods before checking the protocol version.
func socksFixtureGreeting(reader *bufio.Reader, connection net.Conn, header []byte) error {
	if _, err := io.ReadFull(reader, header); err != nil {
		return err
	}
	methods := make([]byte, int(header[1]))
	if _, err := io.ReadFull(reader, methods); err != nil {
		return err
	}
	if header[0] != 5 {
		return fmt.Errorf("invalid SOCKS version")
	}
	connection.Write([]byte{5, 2})
	return nil
}

// socksFixtureAuthentication verifies the exact synthetic credentials after ordered byte reads.
func socksFixtureAuthentication(reader *bufio.Reader, connection net.Conn, header []byte) error {
	if _, err := io.ReadFull(reader, header); err != nil {
		return err
	}
	user := make([]byte, int(header[1]))
	if _, err := io.ReadFull(reader, user); err != nil {
		return err
	}
	length, err := reader.ReadByte()
	if err != nil {
		return err
	}
	password := make([]byte, int(length))
	if _, err := io.ReadFull(reader, password); err != nil {
		return err
	}
	if string(user) != "fixture" || string(password) != "password" {
		return fmt.Errorf("invalid SOCKS credentials")
	}
	connection.Write([]byte{1, 0})
	return nil
}

// socksFixtureDestination validates the DNS address policy after consuming the address and port.
func socksFixtureDestination(reader *bufio.Reader, connection net.Conn, header []byte, scheme string) error {
	request := make([]byte, 4)
	if _, err := io.ReadFull(reader, request); err != nil {
		return err
	}
	address, err := socksFixtureAddress(reader, request[3])
	if err != nil {
		return err
	}
	if _, err := io.ReadFull(reader, header); err != nil {
		return err
	}
	if scheme == "socks5" && request[3] == 3 {
		return fmt.Errorf("local DNS leaked domain")
	}
	if scheme == "socks5h" && (request[3] != 3 || string(address) != "only-proxy-knows.invalid") {
		return fmt.Errorf("remote DNS lost domain")
	}
	connection.Write([]byte{5, 0, 0, 1, 127, 0, 0, 1, 31, 144})
	return nil
}

// socksFixtureAddress reads only the address framing selected by the SOCKS address type.
func socksFixtureAddress(reader *bufio.Reader, addressType byte) ([]byte, error) {
	var address []byte
	switch addressType {
	case 1:
		address = make([]byte, 4)
	case 4:
		address = make([]byte, 16)
	case 3:
		size, err := reader.ReadByte()
		if err != nil {
			return nil, err
		}
		address = make([]byte, int(size))
	default:
		return nil, fmt.Errorf("invalid ATYP")
	}
	if _, err := io.ReadFull(reader, address); err != nil {
		return nil, err
	}
	return address, nil
}
