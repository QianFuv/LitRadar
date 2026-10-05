package index

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

// WorkerProtocolVersion identifies the acknowledged fetch-only process protocol.
const WorkerProtocolVersion uint32 = 8

// WorkerAssignment freezes provider progress and preserves the original catalog ordinal.
type WorkerAssignment struct {
	JournalOrdinal      uint64                     `json:"journal_ordinal"`
	Entry               domain.JournalCatalogEntry `json:"entry"`
	Mode                domain.IndexSyncMode       `json:"mode"`
	CommittedAnchor     *string                    `json:"committed_anchor"`
	TraversalCheckpoint *string                    `json:"traversal_checkpoint"`
}

// Format hides opaque provider recovery payloads.
func (value WorkerAssignment) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "WorkerAssignment{ordinal:%d entry:%v mode:%s has_anchor:%t has_checkpoint:%t}", value.JournalOrdinal, value.Entry, value.Mode, value.CommittedAnchor != nil, value.TraversalCheckpoint != nil)
}

// WorkerRequest is the credential-free request persisted by the parent process.
type WorkerRequest struct {
	ProtocolVersion         uint32             `json:"protocol_version"`
	CatalogName             string             `json:"catalog_name"`
	ProviderName            string             `json:"provider_name"`
	RunId                   string             `json:"run_id"`
	WorkerId                uint64             `json:"worker_id"`
	ProcessCount            uint64             `json:"process_count"`
	SourceWorkerCount       uint64             `json:"source_worker_count"`
	ScheduleEpochUnixMillis uint64             `json:"schedule_epoch_unix_millis"`
	TimeoutSeconds          uint64             `json:"timeout_seconds"`
	Assignments             []WorkerAssignment `json:"assignments"`
}

// Format reports scheduling and correlation metadata without provider state.
func (value WorkerRequest) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "WorkerRequest{version:%d catalog:%q provider:%q run:%q worker:%d processes:%d source_workers:%d epoch:%d timeout:%d assignments:%d credentials:[REDACTED]}", value.ProtocolVersion, value.CatalogName, value.ProviderName, value.RunId, value.WorkerId, value.ProcessCount, value.SourceWorkerCount, value.ScheduleEpochUnixMillis, value.TimeoutSeconds, len(value.Assignments))
}

// WorkerBootstrap carries one-shot secrets only over the owned child stdin pipe.
type WorkerBootstrap struct {
	ProtocolVersion     uint32                `json:"protocol_version"`
	WorkerId            uint64                `json:"worker_id"`
	CnkiCaptchaToken    *string               `json:"cnki_captcha_token,omitempty"`
	ProviderProxyUrl    *string               `json:"provider_proxy_url,omitempty"`
	ScholarlyConfig     *scholarly.LiveConfig `json:"scholarly_config,omitempty"`
	ScholarlyWorksetDir *string               `json:"scholarly_workset_dir,omitempty"`
}

// Format never includes credentials, proxy URLs or workset paths.
func (value WorkerBootstrap) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "WorkerBootstrap{version:%d worker:%d credentials:[REDACTED]}", value.ProtocolVersion, value.WorkerId)
}

// WorkerFailureClass is a redacted error family crossing the private worker boundary.
type WorkerFailureClass string

// WorkerOperation names the safe operation boundary of a worker failure.
type WorkerOperation string

// WorkerFailure excludes provider messages, paths, response bodies and secrets.
type WorkerFailure struct {
	Class              WorkerFailureClass `json:"class"`
	Operation          WorkerOperation    `json:"operation"`
	SqliteCode         *string            `json:"sqlite_code"`
	SqliteExtendedCode *int32             `json:"sqlite_extended_code"`
	IsBusyOrLocked     bool               `json:"is_busy_or_locked"`
}

// WorkerMessage contains one page awaiting ACK or the terminal result after all acknowledged pages.
type WorkerMessage struct {
	Type                                          string
	ProtocolVersion                               uint32
	WorkerId, Sequence, JournalOrdinal, PageIndex uint64
	Batch                                         *domain.ProviderBatch
	Failure                                       *WorkerFailure
}

// ParentMessage acknowledges the exact page after both durable commits have succeeded.
type ParentMessage struct {
	Type            string `json:"type"`
	ProtocolVersion uint32 `json:"protocol_version"`
	WorkerId        uint64 `json:"worker_id"`
	Sequence        uint64 `json:"sequence"`
	JournalOrdinal  uint64 `json:"journal_ordinal"`
	PageIndex       uint64 `json:"page_index"`
	IsComplete      bool   `json:"is_complete"`
}

// ProtocolError classifies framing without exposing raw protocol payloads.
type ProtocolError struct {
	Kind  string
	Cause error
}

func (err *ProtocolError) Error() string {
	switch err.Kind {
	case "end":
		return "worker protocol stream ended"
	case "io":
		return "worker protocol I/O failed"
	default:
		return "worker protocol JSON failed"
	}
}
func (err *ProtocolError) Unwrap() error { return err.Cause }

// ProtocolReader retains buffered bytes across consecutive JSON values without a line-size limit.
type ProtocolReader struct{ decoder *json.Decoder }

// NewProtocolReader attaches a single decoder to a stream for its whole lifetime.
func NewProtocolReader(reader io.Reader) *ProtocolReader {
	return &ProtocolReader{json.NewDecoder(reader)}
}

// Read preserves the original EOF classification for empty and truncated values.
func (reader *ProtocolReader) Read(target any) error {
	err := reader.decoder.Decode(target)
	if err == nil {
		return nil
	}
	if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
		return &ProtocolError{Kind: "end"}
	}
	return &ProtocolError{Kind: "json", Cause: err}
}

// WriteProtocol includes the newline in the JSON write so a peer can close after decoding,
// then flushes the caller's buffer when present.
func WriteProtocol(writer io.Writer, message any) error {
	var buffer bytes.Buffer
	encoder := json.NewEncoder(&buffer)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(message); err != nil {
		return &ProtocolError{Kind: "json", Cause: err}
	}
	body := buffer.Bytes()
	if count, err := writer.Write(body); err != nil {
		return &ProtocolError{Kind: "json", Cause: err}
	} else if count != len(body) {
		return &ProtocolError{Kind: "json", Cause: io.ErrShortWrite}
	}
	if buffered, ok := writer.(interface{ Flush() error }); ok {
		if err := buffered.Flush(); err != nil {
			return &ProtocolError{Kind: "io", Cause: err}
		}
	}
	return nil
}

// MarshalJSON includes only fields belonging to the selected protocol variant.
func (value WorkerMessage) MarshalJSON() ([]byte, error) {
	return value.marshalVariant()
}
func (value WorkerMessage) marshalVariant() ([]byte, error) {
	fields := map[string]any{"type": value.Type, "protocol_version": value.ProtocolVersion, "worker_id": value.WorkerId, "sequence": value.Sequence}
	switch value.Type {
	case "batch":
		if value.Batch == nil {
			return nil, errWorkerJson
		}
		fields["journal_ordinal"] = value.JournalOrdinal
		fields["page_index"] = value.PageIndex
		fields["batch"] = value.Batch
	case "succeeded":
	case "failed":
		if value.Failure == nil {
			return nil, errWorkerJson
		}
		fields["failure"] = value.Failure
	default:
		return nil, errWorkerJson
	}
	return json.Marshal(fields)
}
