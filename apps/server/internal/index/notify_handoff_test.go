package index

import (
	"bytes"
	"errors"
	"io"

	"testing"
)

type protocolFailureWriter struct{ cause error }

func (writer protocolFailureWriter) Write(body []byte) (int, error) {
	return len(body) - 1, writer.cause
}

func TestProtocolWriteFailuresAreReturned(t *testing.T) {
	for _, cause := range []error{nil, io.ErrClosedPipe} {
		err := WriteProtocol(protocolFailureWriter{cause: cause}, ParentMessage{Type: "committed"})
		expected := cause
		if expected == nil {
			expected = io.ErrShortWrite
		}
		if !errors.Is(err, expected) {
			t.Fatalf("error=%v, expected cause=%v", err, expected)
		}
	}
}

type protocolFlushFailureWriter struct{ bytes.Buffer }

func (writer *protocolFlushFailureWriter) Flush() error { return io.ErrClosedPipe }

func TestProtocolFlushFailureIsIo(t *testing.T) {
	err := WriteProtocol(&protocolFlushFailureWriter{}, ParentMessage{Type: "committed"})
	var protocol *ProtocolError
	if !errors.As(err, &protocol) || protocol.Kind != "io" || !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("error=%v", err)
	}
}
