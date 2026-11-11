//go:build go1.27

package index

import (
	"encoding/json"
	jsonv2 "encoding/json/v2"
	"errors"
	"slices"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

var errWorkerJson = errors.New("invalid worker protocol JSON")

func decodeWorkerStruct(body []byte, target any, defaultTail int) error {
	return decodeWorkerFields(body, target, defaultTail, false)
}

func decodeWorkerFields(body []byte, target any, defaultTail int, shouldIgnoreUnknown bool) error {
	if jsonobject.DecodeWithOptions(body, target, jsonobject.Options{AllowUnknown: shouldIgnoreUnknown, OptionalTail: defaultTail}) != nil {
		return errWorkerJson
	}
	return nil
}

// UnmarshalJSON preserves complete typed assignment objects.
func (value *WorkerAssignment) UnmarshalJSON(body []byte) error {
	type plain WorkerAssignment
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 0); err != nil {
		return err
	}
	*value = WorkerAssignment(decoded)
	return nil
}

// UnmarshalJSON rejects unknown, missing and incorrectly typed request fields.
func (value *WorkerRequest) UnmarshalJSON(body []byte) error {
	type plain WorkerRequest
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 0); err != nil {
		return err
	}
	*value = WorkerRequest(decoded)
	return nil
}

// UnmarshalJSON permits omitted bootstrap credential fields.
func (value *WorkerBootstrap) UnmarshalJSON(body []byte) error {
	type plain WorkerBootstrap
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 4); err != nil {
		return err
	}
	*value = WorkerBootstrap(decoded)
	return nil
}

// UnmarshalJSON retains strict typed redacted failures.
func (value *WorkerFailure) UnmarshalJSON(body []byte) error {
	type plain WorkerFailure
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 0); err != nil {
		return err
	}
	*value = WorkerFailure(decoded)
	return nil
}

func decodeWorkerEnum(body []byte, variants []string) (string, error) {
	var value string
	if jsonv2.Unmarshal(body, &value) != nil || !slices.Contains(variants, value) {
		return "", errWorkerJson
	}
	return value, nil
}

// UnmarshalJSON accepts supported enum strings.
func (value *WorkerFailureClass) UnmarshalJSON(body []byte) error {
	decoded, err := decodeWorkerEnum(body, []string{"io", "json", "catalog", "sqlite", "content", "control", "registry", "provider_setup", "provider", "invalid_config", "worker", "notify", "heartbeat"})
	if err == nil {
		*value = WorkerFailureClass(decoded)
	}
	return err
}

// UnmarshalJSON prevents arbitrary provider error text from becoming a protocol operation.
func (value *WorkerOperation) UnmarshalJSON(body []byte) error {
	decoded, err := decodeWorkerEnum(body, []string{"file_system", "worker_json", "catalog_read", "content_database_open", "content_commit", "checkpoint_commit", "control_database", "heartbeat", "provider_registry", "provider_setup", "provider_request", "configuration", "worker_process", "worker_protocol", "notification"})
	if err == nil {
		*value = WorkerOperation(decoded)
	}
	return err
}

func workerVariant(body []byte) (string, []byte, error) {
	var fields map[string]json.RawMessage
	if jsonv2.Unmarshal(body, &fields) != nil || fields == nil {
		return "", nil, errWorkerJson
	}
	var tag string
	if jsonv2.Unmarshal(fields["type"], &tag) != nil {
		return "", nil, errWorkerJson
	}
	delete(fields, "type")
	encoded, err := jsonv2.Marshal(fields)
	return tag, encoded, err
}

// UnmarshalJSON decodes only the fields of the selected internally tagged message, using typed objects.
func (value *WorkerMessage) UnmarshalJSON(body []byte) error {
	tag, body, err := workerVariant(body)
	if err != nil {
		return err
	}
	decoded := WorkerMessage{Type: tag}
	switch tag {
	case "batch":
		var fields struct {
			Version  uint32               `json:"protocol_version"`
			Worker   uint64               `json:"worker_id"`
			Sequence uint64               `json:"sequence"`
			Ordinal  uint64               `json:"journal_ordinal"`
			Page     uint64               `json:"page_index"`
			Batch    domain.ProviderBatch `json:"batch"`
		}
		if err := decodeWorkerStruct(body, &fields, 0); err != nil {
			return err
		}
		decoded.ProtocolVersion = fields.Version
		decoded.WorkerId = fields.Worker
		decoded.Sequence = fields.Sequence
		decoded.JournalOrdinal = fields.Ordinal
		decoded.PageIndex = fields.Page
		decoded.Batch = &fields.Batch
	case "succeeded":
		var fields struct {
			Version  uint32 `json:"protocol_version"`
			Worker   uint64 `json:"worker_id"`
			Sequence uint64 `json:"sequence"`
		}
		if err := decodeWorkerStruct(body, &fields, 0); err != nil {
			return err
		}
		decoded.ProtocolVersion = fields.Version
		decoded.WorkerId = fields.Worker
		decoded.Sequence = fields.Sequence
	case "failed":
		var fields struct {
			Version  uint32        `json:"protocol_version"`
			Worker   uint64        `json:"worker_id"`
			Sequence uint64        `json:"sequence"`
			Failure  WorkerFailure `json:"failure"`
		}
		if err := decodeWorkerStruct(body, &fields, 0); err != nil {
			return err
		}
		decoded.ProtocolVersion = fields.Version
		decoded.WorkerId = fields.Worker
		decoded.Sequence = fields.Sequence
		decoded.Failure = &fields.Failure
	default:
		return errWorkerJson
	}
	*value = decoded
	return nil
}

// UnmarshalJSON accepts only complete commit acknowledgement messages.
func (value *ParentMessage) UnmarshalJSON(body []byte) error {
	tag, body, err := workerVariant(body)
	if err != nil || tag != "committed" {
		return errWorkerJson
	}
	var fields struct {
		Version  uint32 `json:"protocol_version"`
		Worker   uint64 `json:"worker_id"`
		Sequence uint64 `json:"sequence"`
		Ordinal  uint64 `json:"journal_ordinal"`
		Page     uint64 `json:"page_index"`
		Complete bool   `json:"is_complete"`
	}
	if err := decodeWorkerStruct(body, &fields, 0); err != nil {
		return err
	}
	*value = ParentMessage{tag, fields.Version, fields.Worker, fields.Sequence, fields.Ordinal, fields.Page, fields.Complete}
	return nil
}
