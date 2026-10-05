package index

import (
	"errors"
	"io"

	"testing"
)

type separatorFailureWriter struct{ hasWrittenBody bool }

func (writer *separatorFailureWriter) Write(body []byte) (int, error) {
	if writer.hasWrittenBody {
		return 0, io.ErrClosedPipe
	}
	writer.hasWrittenBody = true
	return len(body), nil
}
func TestProtocolSeparatorFailureIsIo(t *testing.T) {
	err := WriteProtocol(&separatorFailureWriter{}, ParentMessage{Type: "committed"})
	var protocol *ProtocolError
	if !errors.As(err, &protocol) || protocol.Kind != "io" {
		t.Fatalf("error=%v", err)
	}
}
