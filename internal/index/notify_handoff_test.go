package index

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"reflect"
	"testing"
)

func TestOriginalRustNotifyHandoff(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/index/notify-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name, Attempt, Database string
				Bytes                   []byte
				Dry                     bool
				Exit                    *int32
			}
			Expected struct {
				Status   string
				Exit     *int32
				Retained int
				Exceeded bool
			}
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, item := range corpus.Observations {
		t.Run(item.Input.Name, func(t *testing.T) {
			reader := bytes.NewReader(item.Input.Bytes)
			retained, exceeded, err := readNotifyOutput(reader)
			if err != nil {
				t.Fatal(err)
			}
			observed := ClassifyNotifyOutput(retained, exceeded, item.Input.Attempt, item.Input.Database, item.Input.Dry, item.Input.Exit)
			if reader.Len() != 0 || len(retained) != item.Expected.Retained || exceeded != item.Expected.Exceeded || string(observed.Status) != item.Expected.Status || !reflect.DeepEqual(observed.ExitCode, item.Expected.Exit) {
				t.Fatalf("status=%s retained=%d exceeded=%t exit=%v remaining=%d", observed.Status, len(retained), exceeded, observed.ExitCode, reader.Len())
			}
		})
	}
}

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
