package runtime

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestDrainHttpForcesNetworkWhileHandlerCloseIsBlocked(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	handlerDone := make(chan struct{})
	handlers := &networkHandlers{next: http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		defer close(handlerDone)
		io.Copy(io.Discard, request.Body)
	})}
	server := &http.Server{Handler: handlers}
	served := make(chan error, 1)
	go func() { served <- server.Serve(listener) }()
	defer server.Close()
	connection, err := net.Dial("tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	awaitBlockedDrainRequest(t, connection)
	handlers.stop()
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	closed := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		done <- drainHttp(ctx, server, func() error { <-handlerDone; close(closed); return nil }, handlers)
	}()
	select {
	case err := <-done:
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatal("drain deadline was lost", err)
		}
	case <-time.After(time.Second):
		connection.Close()
		<-done
		t.Fatal("synchronous closer blocked forced network close")
	}
	select {
	case <-closed:
	default:
		t.Fatal("handler closer was not joined")
	}
	if err := <-served; !errors.Is(err, http.ErrServerClosed) {
		t.Fatal(err)
	}
}

func TestHttpShutdownForcesUnfinishedPostWithinNetworkBudget(t *testing.T) {
	prepared, err := Prepare(context.Background(), runtimeConfiguration(t))
	if err != nil {
		t.Fatal(err)
	}
	defer prepared.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- prepared.runHttp(ctx) }()
	connection, err := net.Dial("tcp", prepared.listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	awaitUnfinishedLoginBody(t, connection)
	started := time.Now()
	cancel()
	select {
	case err := <-done:
		t.Logf("unfinished POST drain: %s", time.Since(started))
		if err != nil && !errors.Is(err, context.DeadlineExceeded) {
			t.Fatal(err)
		}
	case <-time.After(32 * time.Second):
		connection.Close()
		select {
		case <-done:
		case <-time.After(3 * time.Second):
			t.Fatal("shutdown did not join after test socket release")
		}
		t.Fatal("unfinished POST exceeded 30s network drain budget")
	}
}

// awaitBlockedDrainRequest checks that the original incomplete POST started before draining.
func awaitBlockedDrainRequest(t *testing.T, connection net.Conn) {
	t.Helper()
	connection.SetDeadline(time.Now().Add(3 * time.Second))
	if _, err := fmt.Fprint(connection, "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n"); err != nil {
		t.Fatal(err)
	}
	status, err := bufio.NewReader(connection).ReadString('\n')
	if err != nil || !strings.Contains(status, "100 Continue") {
		t.Fatal(status, err)
	}
}

// awaitUnfinishedLoginBody checks the login body read started before cancellation timing.
func awaitUnfinishedLoginBody(t *testing.T, connection net.Conn) {
	t.Helper()
	connection.SetDeadline(time.Now().Add(40 * time.Second))
	if _, err := fmt.Fprintf(connection, "POST /api/auth/login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n"); err != nil {
		t.Fatal(err)
	}
	reader := bufio.NewReader(connection)
	status, err := reader.ReadString('\n')
	if err != nil || !strings.Contains(status, "100 Continue") {
		t.Fatal("body read did not start", status, err)
	}
}
