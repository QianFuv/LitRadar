package transport

import (
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"syscall"
	"testing"
)

// isExpectedCloseError recognizes only completed connection shutdown and peer disconnects.
// Every component of a joined error must be expected; unrelated failures remain visible.
func isExpectedCloseError(err error) bool {
	if err == nil {
		return true
	}
	if joined, ok := err.(interface{ Unwrap() []error }); ok {
		children := joined.Unwrap()
		if len(children) == 0 {
			return false
		}
		for _, child := range children {
			if !isExpectedCloseError(child) {
				return false
			}
		}
		return true
	}
	if inner := errors.Unwrap(err); inner != nil {
		return isExpectedCloseError(inner)
	}
	return errors.Is(err, net.ErrClosed) || errors.Is(err, io.EOF) ||
		errors.Is(err, syscall.ECONNRESET) || errors.Is(err, syscall.ECONNABORTED) ||
		errors.Is(err, syscall.EPIPE) || errors.Is(err, syscall.ENOTCONN) || errors.Is(err, syscall.ESHUTDOWN) ||
		isPlatformPeerCloseError(err)
}

func TestExpectedCloseErrorsAreNarrowAndUnwrapTLSFailures(t *testing.T) {
	for _, err := range []error{nil, io.EOF, net.ErrClosed, syscall.ECONNRESET, syscall.ECONNABORTED, syscall.EPIPE, syscall.ENOTCONN, syscall.ESHUTDOWN} {
		if !isExpectedCloseError(err) {
			t.Fatalf("expected shutdown rejected: %v", err)
		}
		if err != nil {
			wrapped := fmt.Errorf("tls: failed to send closeNotify alert: %w", &net.OpError{Op: "write", Net: "tcp", Err: os.NewSyscallError("send", err)})
			if !isExpectedCloseError(wrapped) {
				t.Fatal(wrapped)
			}
		}
	}
	for _, err := range []error{os.ErrDeadlineExceeded, os.ErrPermission, io.ErrUnexpectedEOF, syscall.ECONNREFUSED, syscall.EBADF, errors.New("connection reset by peer"), errors.Join(syscall.ECONNRESET, os.ErrPermission)} {
		if isExpectedCloseError(err) {
			t.Fatalf("unexpected error hidden: %v", err)
		}
	}
}
