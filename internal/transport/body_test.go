package transport

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"strings"
	"testing"
	"time"

	platform "github.com/QianFuv/LitRadar/internal/platform/transport"
)

func rawResponse(t *testing.T, headers string, body []byte) *http.Response {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { listener.Close() })
	complete := make(chan error, 1)
	go func() {
		connection, err := listener.Accept()
		if err != nil {
			complete <- err
			return
		}
		defer connection.Close()
		connection.SetDeadline(time.Now().Add(3 * time.Second))
		request, err := http.ReadRequest(bufio.NewReader(connection))
		if err != nil {
			complete <- err
			return
		}
		request.Body.Close()
		_, err = fmt.Fprintf(connection, "HTTP/1.1 200 OK\r\n%sConnection: close\r\n\r\n", headers)
		if err == nil {
			_, err = connection.Write(body)
		}
		complete <- err
	}()
	wire, err := platform.New("")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(wire.CloseIdleConnections)
	client := http.Client{Transport: wire, Timeout: 3 * time.Second}
	response, err := client.Get("http://" + listener.Addr().String() + "/body")
	if err != nil {
		t.Fatal(err)
	}
	if err := <-complete; err != nil {
		response.Body.Close()
		t.Fatal(err)
	}
	return response
}

func TestDecodedBodyLimitsOverRealHttp(t *testing.T) {
	var compressed bytes.Buffer
	writer := gzip.NewWriter(&compressed)
	writer.Write(bytes.Repeat([]byte("a"), 256))
	writer.Close()
	for _, test := range []struct {
		name, headers string
		body          []byte
		want          error
	}{
		{"length", "Content-Length: 65\r\n", bytes.Repeat([]byte("a"), 65), ErrTooLarge},
		{"chunked", "Transfer-Encoding: chunked\r\n", []byte("41\r\n" + strings.Repeat("a", 65) + "\r\n0\r\n\r\n"), ErrTooLarge},
		{"gzip", "Content-Encoding: gzip\r\nContent-Length: " + strconv.Itoa(compressed.Len()) + "\r\n", compressed.Bytes(), ErrTooLarge},
		{"truncated", "Content-Length: 32\r\n", []byte(`{"ok":true}`), ErrReadFailed},
		{"invalid-json", "Content-Length: 8\r\n", []byte("not-json"), ErrInvalidJson},
		{"valid", "Content-Length: 11\r\n", []byte(`{"ok":true}`), nil},
	} {
		t.Run(test.name, func(t *testing.T) {
			_, err := BoundedJson(rawResponse(t, test.headers, test.body), 64)
			if !errors.Is(err, test.want) {
				t.Fatalf("got %v want %v", err, test.want)
			}
		})
	}
}

type failedBody struct{ hasRead, hasClosed bool }

func (body *failedBody) Read(output []byte) (int, error) {
	body.hasRead = true
	return copy(output, []byte("12345")), errors.New("private upstream failure")
}
func (body *failedBody) Close() error { body.hasClosed = true; return nil }

func TestReadFailureAndLengthErrorPriority(t *testing.T) {
	for _, length := range []int64{-1, 99} {
		body := &failedBody{}
		_, err := BoundedBytes(&http.Response{ContentLength: length, Body: body}, 4)
		want := ErrReadFailed
		if length > 4 {
			want = ErrTooLarge
		}
		if err != want || !body.hasClosed || body.hasRead != (length < 4) {
			t.Fatalf("length %d: %v %+v", length, err, body)
		}
	}
}

func TestLossyTextIgnoresCharsetAndPreservesBom(t *testing.T) {
	response := &http.Response{ContentLength: -1, Header: http.Header{"Content-Type": []string{"text/plain; charset=windows-1252"}}, Body: io.NopCloser(bytes.NewReader([]byte{0xef, 0xbb, 0xbf, 0xe6, 0x83, 0xff, 0xff}))}
	actual, err := BoundedText(response, 16)
	if err != nil || actual != "\ufeff\ufffd\ufffd\ufffd" {
		t.Fatalf("%q %v", actual, err)
	}
}

func TestJsonChecksEveryNumberBeforeDiscardingDuplicateValues(t *testing.T) {
	for _, input := range []string{`{"x":1e400,"x":1}`, `{"x":[1e400],"x":1}`, `{"ignored":2e308}`, `{"x":"\ud800","x":0}`, `1 2`} {
		if _, err := ParseJson([]byte(input)); err != ErrInvalidJson {
			t.Fatalf("%s: %v", input, err)
		}
	}
	for _, input := range []string{`{"x":-0e999999999999999999999999999999999999999}`, `{"x":1e-999999999999999999999999999999999999999}`, `{"x":1e308}`, `{"x":"1e400","quoted":"\\\"1e400"}`} {
		if _, err := ParseJson([]byte(input)); err != nil {
			t.Fatalf("%s: %v", input, err)
		}
	}
}
