package sources

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

var errContractJson = errors.New("invalid canonical provider contract JSON")

// decodeContractStruct validates JSON before decoding fields in their source order.
func decodeContractStruct(data []byte, target any) error {
	if !jsonvalue.ValidJson(string(data)) {
		return errContractJson
	}
	value := reflect.ValueOf(target).Elem()
	kind := value.Type()
	data = bytes.TrimSpace(data)
	fields := contractFields(kind)
	seen := make([]bool, kind.NumField())
	var err error
	switch data[0] {
	case '[':
		err = decodeContractSequence(data, value, seen)
	case '{':
		err = decodeContractObject(data, value, fields, seen)
	default:
		return errContractJson
	}
	if err != nil {
		return err
	}
	return requireContractFields(kind, seen)
}

// contractFields retains the declared JSON-name to field-position mapping.
func contractFields(kind reflect.Type) map[string]int {
	fields := make(map[string]int, kind.NumField())
	for index := 0; index < kind.NumField(); index++ {
		name, _, _ := strings.Cut(kind.Field(index).Tag.Get("json"), ",")
		fields[name] = index
	}
	return fields
}

// decodeContractSequence requires exact arity before mutating declared fields.
func decodeContractSequence(data []byte, value reflect.Value, seen []bool) error {
	var sequence []json.RawMessage
	if err := json.Unmarshal(data, &sequence); err != nil || len(sequence) != value.NumField() {
		return errContractJson
	}
	for index, raw := range sequence {
		if err := decodeContractValue(raw, value.Field(index)); err != nil {
			return err
		}
		seen[index] = true
	}
	return nil
}

// decodeContractObject rejects unknown or duplicate names before decoding their values.
func decodeContractObject(data []byte, value reflect.Value, fields map[string]int, seen []bool) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.Token()
	for decoder.More() {
		key, err := decoder.Token()
		if err != nil {
			return errContractJson
		}
		index, exists := fields[key.(string)]
		if !exists || seen[index] {
			return errContractJson
		}
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			return errContractJson
		}
		if err := decodeContractValue(raw, value.Field(index)); err != nil {
			return err
		}
		seen[index] = true
	}
	return nil
}

// requireContractFields permits absent pointer fields and requires every other field.
func requireContractFields(kind reflect.Type, seen []bool) error {
	for index, hasField := range seen {
		if !hasField && kind.Field(index).Type.Kind() != reflect.Pointer {
			return errContractJson
		}
	}
	return nil
}

// decodeContractValue preserves pointer allocation and null rejection before typed decoding.
func decodeContractValue(raw []byte, value reflect.Value) error {
	raw = bytes.TrimSpace(raw)
	if value.Kind() == reflect.Pointer {
		if bytes.Equal(raw, []byte("null")) {
			value.SetZero()
			return nil
		}
		value.Set(reflect.New(value.Type().Elem()))
		return decodeContractValue(raw, value.Elem())
	}
	if bytes.Equal(raw, []byte("null")) {
		return errContractJson
	}
	if value.Kind() == reflect.Slice {
		return decodeContractSlice(raw, value)
	}
	if value.Kind() == reflect.Int64 {
		return decodeContractInteger(raw, value)
	}
	if err := json.Unmarshal(raw, value.Addr().Interface()); err != nil {
		return errContractJson
	}
	return nil
}

func encodeContractStruct(value any) ([]byte, error) {
	copy := reflect.New(reflect.TypeOf(value)).Elem()
	copy.Set(reflect.ValueOf(value))
	for index := 0; index < copy.NumField(); index++ {
		field := copy.Field(index)
		if field.Kind() == reflect.Slice && field.IsNil() {
			field.Set(reflect.MakeSlice(field.Type(), 0, 0))
		}
	}
	return json.Marshal(copy.Interface())
}

// UnmarshalJSON enforces required, duplicate, unknown and sequence-field rules.
func (value *JournalRankings) UnmarshalJSON(data []byte) error {
	type wire JournalRankings
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalRankings(decoded)
	return nil
}

// UnmarshalJSON preserves the maintained catalog's strict derived-serde contract.
func (value *JournalCatalogEntry) UnmarshalJSON(data []byte) error {
	type wire JournalCatalogEntry
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalCatalogEntry(decoded)
	return nil
}

// MarshalJSON emits required empty collections as arrays, never null.
func (value JournalCatalogEntry) MarshalJSON() ([]byte, error) {
	type wire JournalCatalogEntry
	return encodeContractStruct(wire(value))
}

// UnmarshalJSON preserves strict journal observation fields and sequence representations.
func (value *JournalDraft) UnmarshalJSON(data []byte) error {
	type wire JournalDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalDraft(decoded)
	return nil
}

// MarshalJSON retains empty observed collections as arrays.
func (value JournalDraft) MarshalJSON() ([]byte, error) {
	type wire JournalDraft
	return encodeContractStruct(wire(value))
}

// UnmarshalJSON rejects invalid issue field types, including integral floating-point years.
func (value *IssueDraft) UnmarshalJSON(data []byte) error {
	type wire IssueDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = IssueDraft(decoded)
	return nil
}

// UnmarshalJSON validates one required canonical author field.
func (value *ArticleAuthorDraft) UnmarshalJSON(data []byte) error {
	type wire ArticleAuthorDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ArticleAuthorDraft(decoded)
	return nil
}

// UnmarshalJSON enforces the complete typed article contract before content validation.
func (value *ArticleDraft) UnmarshalJSON(data []byte) error {
	type wire ArticleDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ArticleDraft(decoded)
	return nil
}

// MarshalJSON preserves the required author and retraction collections.
func (value ArticleDraft) MarshalJSON() ([]byte, error) {
	type wire ArticleDraft
	return encodeContractStruct(wire(value))
}

// UnmarshalJSON validates nested batches before they can reach persistence.
func (value *ProviderBatch) UnmarshalJSON(data []byte) error {
	type wire ProviderBatch
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ProviderBatch(decoded)
	return nil
}

// MarshalJSON emits an empty content page using the original array shape.
func (value ProviderBatch) MarshalJSON() ([]byte, error) {
	type wire ProviderBatch
	return encodeContractStruct(wire(value))
}

// UnmarshalJSON rejects synchronization modes outside the frozen enum.
func (value *IndexSyncMode) UnmarshalJSON(data []byte) error {
	var decoded string
	if !jsonvalue.ValidJson(string(data)) {
		return errContractJson
	}
	data = bytes.TrimSpace(data)
	if data[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		if !decoder.More() {
			return errContractJson
		}
		key, err := decoder.Token()
		if err != nil {
			return errContractJson
		}
		decoded = key.(string)
		var unit json.RawMessage
		if decoder.Decode(&unit) != nil || !bytes.Equal(bytes.TrimSpace(unit), []byte("null")) || decoder.More() {
			return errContractJson
		}
	} else if json.Unmarshal(data, &decoded) != nil {
		return errContractJson
	}
	switch IndexSyncMode(decoded) {
	case Bootstrap, Incremental, FullRescan:
		*value = IndexSyncMode(decoded)
		return nil
	}
	return errContractJson
}

// UnmarshalJSON enforces variant-specific fields and required continuation checkpoints.
func (value *ProviderProgress) UnmarshalJSON(data []byte) error {
	if !jsonvalue.ValidJson(string(data)) {
		return errContractJson
	}
	data = bytes.TrimSpace(data)
	if data[0] == '[' {
		return decodeProgressSequence(data, value)
	}
	var selector struct {
		State string `json:"state"`
	}
	if json.Unmarshal(data, &selector) != nil {
		return errContractJson
	}
	switch ProgressState(selector.State) {
	case Continue:
		var wire struct {
			State      string `json:"state"`
			Checkpoint string `json:"checkpoint"`
		}
		if err := decodeContractStruct(data, &wire); err != nil {
			return err
		}
		*value = ProviderProgress{State: Continue, Checkpoint: &wire.Checkpoint}
		return nil
	case Complete:
		var wire struct {
			State      string  `json:"state"`
			NextAnchor *string `json:"next_anchor,omitempty"`
		}
		if err := decodeContractStruct(data, &wire); err != nil {
			return err
		}
		*value = ProviderProgress{State: Complete, NextAnchor: wire.NextAnchor}
		return nil
	default:
		return errContractJson
	}
}

// MarshalJSON writes only the fields of the selected provider progress variant.
func (value ProviderProgress) MarshalJSON() ([]byte, error) {
	switch value.State {
	case Continue:
		if value.Checkpoint == nil || value.NextAnchor != nil {
			return nil, errContractJson
		}
		return json.Marshal(struct {
			State      ProgressState `json:"state"`
			Checkpoint string        `json:"checkpoint"`
		}{value.State, *value.Checkpoint})
	case Complete:
		if value.Checkpoint != nil {
			return nil, errContractJson
		}
		return json.Marshal(struct {
			State      ProgressState `json:"state"`
			NextAnchor *string       `json:"next_anchor,omitempty"`
		}{value.State, value.NextAnchor})
	default:
		return nil, errContractJson
	}
}

// decodeContractSlice allocates the entire sequence before decoding children in order.
func decodeContractSlice(raw []byte, value reflect.Value) error {
	var items []json.RawMessage
	if err := json.Unmarshal(raw, &items); err != nil {
		return errContractJson
	}
	value.Set(reflect.MakeSlice(value.Type(), len(items), len(items)))
	for index, item := range items {
		if err := decodeContractValue(item, value.Index(index)); err != nil {
			return err
		}
	}
	return nil
}

// decodeContractInteger accepts only representable integral numeric variants.
func decodeContractInteger(raw []byte, value reflect.Value) error {
	number, err := ParseNumber(json.Number(raw))
	if err != nil {
		return errContractJson
	}
	signed, hasSigned := number.AsInt64()
	if !hasSigned {
		return errContractJson
	}
	value.SetInt(signed)
	return nil
}

// decodeProgressSequence preserves variant arity, null admission and assignment timing.
func decodeProgressSequence(data []byte, value *ProviderProgress) error {
	var sequence []json.RawMessage
	if json.Unmarshal(data, &sequence) != nil || len(sequence) < 1 || len(sequence) > 2 {
		return errContractJson
	}
	var state ProgressState
	if json.Unmarshal(sequence[0], &state) != nil {
		return errContractJson
	}
	switch state {
	case Continue:
		return decodeProgressContinuation(sequence, value)
	case Complete:
		return decodeProgressCompletion(sequence, value)
	default:
		return errContractJson
	}

}

// decodeProgressContinuation requires a nonnull checkpoint before replacing progress.
func decodeProgressContinuation(sequence []json.RawMessage, value *ProviderProgress) error {
	if len(sequence) != 2 || bytes.Equal(bytes.TrimSpace(sequence[1]), []byte("null")) {
		return errContractJson
	}
	var checkpoint string
	if json.Unmarshal(sequence[1], &checkpoint) != nil {
		return errContractJson
	}
	*value = ProviderProgress{State: Continue, Checkpoint: &checkpoint}
	return nil
}

// decodeProgressCompletion permits an absent or null anchor before replacing progress.
func decodeProgressCompletion(sequence []json.RawMessage, value *ProviderProgress) error {
	var anchor *string
	if len(sequence) == 2 && json.Unmarshal(sequence[1], &anchor) != nil {
		return errContractJson
	}
	*value = ProviderProgress{State: Complete, NextAnchor: anchor}
	return nil
}
