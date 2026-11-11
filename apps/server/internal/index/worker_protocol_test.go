package index

import (
	"fmt"
	"io"

	"strings"
	"testing"
	"time"
)

type peerClosingWriter struct {
	writer *io.PipeWriter
	closed <-chan struct{}
}

func (writer peerClosingWriter) Write(body []byte) (int, error) {
	count, err := writer.writer.Write(body)
	<-writer.closed
	return count, err
}

func TestFinalAckSucceedsWhenWorkerClosesAfterDecode(t *testing.T) {
	reader, writer := io.Pipe()
	defer func() {
		if err := reader.Close(); err != nil {
			t.Errorf("reader.Close: %v", err)
		}
	}()
	defer func() {
		if err := writer.Close(); err != nil {
			t.Errorf("writer.Close: %v", err)
		}
	}()
	timer := time.AfterFunc(5*time.Second, func() { reader.CloseWithError(io.ErrNoProgress) })
	defer timer.Stop()
	closed := make(chan struct{})
	received := make(chan error, 1)
	expected := ParentMessage{Type: "committed", ProtocolVersion: WorkerProtocolVersion, IsComplete: true}
	go func() {
		var actual ParentMessage
		err := NewProtocolReader(reader).Read(&actual)
		if err == nil && actual != expected {
			err = fmt.Errorf("acknowledgement=%+v", actual)
		}
		if err := reader.Close(); err != nil {
			t.Errorf("reader.Close: %v", err)
		}
		close(closed)
		received <- err
	}()
	err := WriteProtocol(peerClosingWriter{writer: writer, closed: closed}, expected)
	if receiveErr := <-received; receiveErr != nil {
		t.Fatal(receiveErr)
	}
	if err != nil {
		t.Fatalf("worker received the final acknowledgement, but parent failed: %v", err)
	}
}

func TestWorkerDiagnosticsExcludeOpaqueState(t *testing.T) {
	secret := "private-token-and-cursor"
	values := []any{WorkerBootstrap{CnkiCaptchaToken: &secret, ProviderProxyUrl: &secret, ScholarlyWorksetDir: &secret}, WorkerAssignment{CommittedAnchor: &secret, TraversalCheckpoint: &secret}, WorkerRequest{Assignments: []WorkerAssignment{{CommittedAnchor: &secret}}}}
	for _, value := range values {
		for _, format := range []string{"%v", "%+v", "%#v"} {
			if strings.Contains(fmt.Sprintf(format, value), secret) {
				t.Fatalf("leaked with %s", format)
			}
		}
	}
}
