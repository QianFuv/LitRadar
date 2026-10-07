package index

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

var errWorkerJson = errors.New("invalid worker protocol JSON")

func decodeWorkerStruct(body []byte, target any, defaultTail int) error {
	return decodeWorkerFields(body, target, defaultTail, false)
}

// decodeWorkerFields validates the complete JSON before mutating reflected fields.
func decodeWorkerFields(body []byte, target any, defaultTail int, shouldIgnoreUnknown bool) error {
	if !jsonvalue.ValidJson(string(body)) {
		return errWorkerJson
	}
	body = bytes.TrimSpace(body)
	value := reflect.ValueOf(target).Elem()
	fields := workerFieldOrdinals(value.Type())
	seen := make([]bool, value.NumField())
	var err error
	switch body[0] {
	case '[':
		err = decodeWorkerSequence(body, value, defaultTail, seen)
	case '{':
		err = decodeWorkerObject(body, value, fields, seen, shouldIgnoreUnknown)
	default:
		return errWorkerJson
	}
	if err != nil {
		return err
	}
	return validateRequiredWorkerFields(value, seen, defaultTail)
}

// workerFieldOrdinals retains the declaration order and JSON field names.
func workerFieldOrdinals(kind reflect.Type) map[string]int {
	fields := map[string]int{}
	for ordinal := 0; ordinal < kind.NumField(); ordinal++ {
		name, _, _ := strings.Cut(kind.Field(ordinal).Tag.Get("json"), ",")
		fields[name] = ordinal
	}
	return fields
}

// decodeWorkerSequence requires positional slots, including nullable fields.
func decodeWorkerSequence(body []byte, value reflect.Value, defaultTail int, seen []bool) error {
	var sequence []json.RawMessage
	if json.Unmarshal(body, &sequence) != nil || len(sequence) > len(seen) || len(sequence) < len(seen)-defaultTail {
		return errWorkerJson
	}
	for ordinal, raw := range sequence {
		if err := decodeWorkerValue(raw, value.Field(ordinal)); err != nil {
			return err
		}
		seen[ordinal] = true
	}
	return nil
}

// decodeWorkerObject rejects duplicate known fields before decoding their values.
func decodeWorkerObject(body []byte, value reflect.Value, fields map[string]int, seen []bool, shouldIgnoreUnknown bool) error {
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.Token()
	for decoder.More() {
		key, err := decoder.Token()
		if err != nil {
			return errWorkerJson
		}
		ordinal, exists := fields[key.(string)]
		if !exists && shouldIgnoreUnknown {
			var ignored json.RawMessage
			if decoder.Decode(&ignored) != nil {
				return errWorkerJson
			}
			continue
		}
		if !exists || seen[ordinal] {
			return errWorkerJson
		}
		if err := decodeWorkerObjectField(decoder, value.Field(ordinal)); err != nil {
			return err
		}
		seen[ordinal] = true
	}
	return nil
}

// decodeWorkerObjectField consumes one value before applying its reflected decoder.
func decodeWorkerObjectField(decoder *json.Decoder, value reflect.Value) error {
	var raw json.RawMessage
	if decoder.Decode(&raw) != nil {
		return errWorkerJson
	}
	return decodeWorkerValue(raw, value)
}

// validateRequiredWorkerFields permits omitted pointers and the defaulted tail in maps.
func validateRequiredWorkerFields(value reflect.Value, seen []bool, defaultTail int) error {
	for ordinal, hasField := range seen {
		if !hasField && value.Field(ordinal).Kind() != reflect.Pointer && ordinal < len(seen)-defaultTail {
			return errWorkerJson
		}
	}
	return nil
}
func decodeWorkerValue(raw []byte, value reflect.Value) error {
	raw = bytes.TrimSpace(raw)
	if bytes.Equal(raw, []byte("null")) {
		if value.Kind() != reflect.Pointer {
			return errWorkerJson
		}
		value.SetZero()
		return nil
	}
	if value.Kind() == reflect.Pointer {
		value.Set(reflect.New(value.Type().Elem()))
		return decodeWorkerValue(raw, value.Elem())
	}
	if value.Kind() == reflect.Slice {
		var sequence []json.RawMessage
		if json.Unmarshal(raw, &sequence) != nil {
			return errWorkerJson
		}
		value.Set(reflect.MakeSlice(value.Type(), len(sequence), len(sequence)))
		for ordinal, item := range sequence {
			if err := decodeWorkerValue(item, value.Index(ordinal)); err != nil {
				return err
			}
		}
		return nil
	}
	if json.Unmarshal(raw, value.Addr().Interface()) != nil {
		return errWorkerJson
	}
	return nil
}

// UnmarshalJSON preserves strict maps and complete positional assignment arrays.
func (value *WorkerAssignment) UnmarshalJSON(body []byte) error {
	type plain WorkerAssignment
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 0); err != nil {
		return err
	}
	*value = WorkerAssignment(decoded)
	return nil
}

// UnmarshalJSON rejects unknown, duplicate, missing and incorrectly typed request fields.
func (value *WorkerRequest) UnmarshalJSON(body []byte) error {
	type plain WorkerRequest
	var decoded plain
	if err := decodeWorkerStruct(body, &decoded, 0); err != nil {
		return err
	}
	*value = WorkerRequest(decoded)
	return nil
}

// UnmarshalJSON permits omitted bootstrap credentials in maps and the explicitly defaulted array tail.
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
	if !jsonvalue.ValidJson(string(body)) {
		return "", errWorkerJson
	}
	body = bytes.TrimSpace(body)
	var value string
	if body[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(body))
		decoder.Token()
		if !decoder.More() {
			return "", errWorkerJson
		}
		key, err := decoder.Token()
		if err != nil {
			return "", errWorkerJson
		}
		value = key.(string)
		var unit json.RawMessage
		if decoder.Decode(&unit) != nil || !bytes.Equal(bytes.TrimSpace(unit), []byte("null")) || decoder.More() {
			return "", errWorkerJson
		}
	} else if json.Unmarshal(body, &value) != nil {
		return "", errWorkerJson
	}
	if !slices.Contains(variants, value) {
		return "", errWorkerJson
	}
	return value, nil
}

// UnmarshalJSON accepts the original unit enum string or single-key object representation.
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

// workerVariant validates JSON before selecting its positional or tagged representation.
func workerVariant(body []byte) (string, []byte, error) {
	if !jsonvalue.ValidJson(string(body)) {
		return "", nil, errWorkerJson
	}
	body = bytes.TrimSpace(body)
	switch body[0] {
	case '[':
		return workerSequenceVariant(body)
	case '{':
		return workerObjectVariant(body)
	default:
		return "", nil, errWorkerJson
	}
}

// workerSequenceVariant removes the string tag while preserving positional payload slots.
func workerSequenceVariant(body []byte) (string, []byte, error) {
	var values []json.RawMessage
	var tag string
	if json.Unmarshal(body, &values) != nil || len(values) == 0 || json.Unmarshal(values[0], &tag) != nil {
		return "", nil, errWorkerJson
	}
	remainder, err := json.Marshal(values[1:])
	return tag, remainder, err
}

// workerObjectVariant retains ordered raw payload values and rejects every duplicate key.
func workerObjectVariant(body []byte) (string, []byte, error) {
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.Token()
	seen := map[string]bool{}
	var tag string
	var remainder bytes.Buffer
	remainder.WriteByte('{')
	hasField := false
	for decoder.More() {
		name, raw, err := readWorkerVariantField(decoder, seen)
		if err != nil {
			return "", nil, err
		}
		if name == "type" {
			if json.Unmarshal(raw, &tag) != nil {
				return "", nil, errWorkerJson
			}
		} else {
			appendWorkerVariantField(&remainder, name, raw, hasField)
			hasField = true
		}
	}
	remainder.WriteByte('}')
	if !seen["type"] {
		return "", nil, errWorkerJson
	}
	return tag, remainder.Bytes(), nil
}

// readWorkerVariantField records a unique field name before consuming its raw value.
func readWorkerVariantField(decoder *json.Decoder, seen map[string]bool) (string, json.RawMessage, error) {
	key, err := decoder.Token()
	if err != nil {
		return "", nil, errWorkerJson
	}
	name := key.(string)
	if seen[name] {
		return "", nil, errWorkerJson
	}
	seen[name] = true
	var raw json.RawMessage
	if decoder.Decode(&raw) != nil {
		return "", nil, errWorkerJson
	}
	return name, raw, nil
}

// appendWorkerVariantField reconstructs JSON without re-encoding the field value.
func appendWorkerVariantField(remainder *bytes.Buffer, name string, raw json.RawMessage, hasField bool) {
	if hasField {
		remainder.WriteByte(',')
	}
	encoded, _ := json.Marshal(name)
	remainder.Write(encoded)
	remainder.WriteByte(':')
	remainder.Write(raw)
}

// UnmarshalJSON decodes only the fields of the selected internally tagged message, including positional form.
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
