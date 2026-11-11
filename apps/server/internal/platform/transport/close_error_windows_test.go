package transport

import (
	"errors"
	"fmt"
	"net"
	"os"
	"testing"

	"golang.org/x/sys/windows"
)

func isPlatformPeerCloseError(err error) bool {
	return errors.Is(err, windows.WSAECONNRESET) || errors.Is(err, windows.WSAECONNABORTED) ||
		errors.Is(err, windows.WSAENOTCONN) || errors.Is(err, windows.WSAESHUTDOWN) || errors.Is(err, windows.WSAENETRESET)
}

func TestExpectedWinsockCloseErrors(t *testing.T) {
	for _, err := range []error{windows.WSAECONNRESET, windows.WSAECONNABORTED, windows.WSAENOTCONN, windows.WSAESHUTDOWN, windows.WSAENETRESET} {
		wrapped := fmt.Errorf("tls: failed to send closeNotify alert (but connection was closed anyway): %w", &net.OpError{Op: "write", Net: "tcp", Err: os.NewSyscallError("wsasend", err)})
		if !isExpectedCloseError(wrapped) {
			t.Fatal(wrapped)
		}
	}
	if isExpectedCloseError(windows.WSAETIMEDOUT) || isExpectedCloseError(windows.WSAEACCES) {
		t.Fatal("unrelated Winsock error hidden")
	}
}
