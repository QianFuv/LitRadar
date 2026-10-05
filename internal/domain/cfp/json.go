package cfp

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
	"slices"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

var errJson = errors.New("invalid CFP payload JSON")

// decodeStruct retains serde's required, nullable, duplicate and positional field contracts.
func decodeStruct(data []byte, target any, deniesUnknown bool) error {
	if !utf8.Valid(data) || !json.Valid(data) {
		return errJson
	}
	value := reflect.ValueOf(target).Elem()
	kind := value.Type()
	data = bytes.TrimSpace(data)
	fields := map[string]int{}
	for index := 0; index < kind.NumField(); index++ {
		fields[kind.Field(index).Tag.Get("json")] = index
	}
	seen := make([]bool, kind.NumField())
	if data[0] == '[' {
		var sequence []json.RawMessage
		if json.Unmarshal(data, &sequence) != nil || len(sequence) > kind.NumField() {
			return errJson
		}
		for index := len(sequence); index < kind.NumField(); index++ {
			if kind.Field(index).Tag.Get("default") != "true" {
				return errJson
			}
		}
		for index, raw := range sequence {
			if err := decodeValue(raw, value.Field(index)); err != nil {
				return err
			}
			seen[index] = true
		}
	} else if data[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		for decoder.More() {
			key, err := decoder.Token()
			if err != nil {
				return errJson
			}
			var raw json.RawMessage
			if decoder.Decode(&raw) != nil {
				return errJson
			}
			index, exists := fields[key.(string)]
			if !exists {
				if deniesUnknown {
					return errJson
				}
				continue
			}
			if seen[index] {
				return errJson
			}
			if err := decodeValue(raw, value.Field(index)); err != nil {
				return err
			}
			seen[index] = true
		}
	} else {
		return errJson
	}
	for index, hasField := range seen {
		if !hasField && kind.Field(index).Type.Kind() != reflect.Pointer {
			if kind.Field(index).Tag.Get("default") != "true" {
				return errJson
			}
			if value.Field(index).Kind() == reflect.Slice {
				value.Field(index).Set(reflect.MakeSlice(value.Field(index).Type(), 0, 0))
			}
		}
	}
	return nil
}

func decodeValue(raw []byte, value reflect.Value) error {
	raw = bytes.TrimSpace(raw)
	if value.Kind() == reflect.Pointer {
		if bytes.Equal(raw, []byte("null")) {
			value.SetZero()
			return nil
		}
		value.Set(reflect.New(value.Type().Elem()))
		return decodeValue(raw, value.Elem())
	}
	if bytes.Equal(raw, []byte("null")) {
		return errJson
	}
	if value.Kind() == reflect.Slice {
		var items []json.RawMessage
		if json.Unmarshal(raw, &items) != nil {
			return errJson
		}
		value.Set(reflect.MakeSlice(value.Type(), len(items), len(items)))
		for index, item := range items {
			if err := decodeValue(item, value.Index(index)); err != nil {
				return err
			}
		}
		return nil
	}
	if value.Kind() != reflect.Struct && !jsonvalue.ValidJson(string(raw)) {
		return errJson
	}
	if json.Unmarshal(raw, value.Addr().Interface()) != nil {
		return errJson
	}
	return nil
}

func encodeStruct(value any) ([]byte, error) {
	copy := reflect.New(reflect.TypeOf(value)).Elem()
	copy.Set(reflect.ValueOf(value))
	for index := 0; index < copy.NumField(); index++ {
		field := copy.Field(index)
		if field.Kind() == reflect.Slice && field.IsNil() {
			field.Set(reflect.MakeSlice(field.Type(), 0, 0))
		}
	}
	encoded, err := jsonvalue.EncodeJson(copy.Interface())
	return []byte(encoded), err
}

func decodeEnum(data []byte, allowed []string) (string, error) {
	if !jsonvalue.ValidJson(string(data)) {
		return "", errJson
	}
	data = bytes.TrimSpace(data)
	var value string
	if data[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		if !decoder.More() {
			return "", errJson
		}
		token, err := decoder.Token()
		if err != nil {
			return "", errJson
		}
		value = token.(string)
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil || !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) || decoder.More() {
			return "", errJson
		}
	} else if json.Unmarshal(data, &value) != nil {
		return "", errJson
	}
	if !slices.Contains(allowed, value) {
		return "", errJson
	}
	return value, nil
}

func (value *Stage) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"paper", "abstract", "proposal", "opens", "revision", "decision", "publication", "event", "registration"})
	if err == nil {
		*value = Stage(decoded)
	}
	return err
}
func (value *Kind) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"special_issue", "general", "proposal", "conference_linked"})
	if err == nil {
		*value = Kind(decoded)
	}
	return err
}
func (value *State) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"open", "upcoming", "closed", "historical", "invitation_only", "undated", "uncertain"})
	if err == nil {
		*value = State(decoded)
	}
	return err
}

func (value *Source) UnmarshalJSON(data []byte) error {
	type wire Source
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = Source(decoded)
	return nil
}
func (value Source) MarshalJSON() ([]byte, error) { type wire Source; return encodeStruct(wire(value)) }
func (value *Seed) UnmarshalJSON(data []byte) error {
	type wire Seed
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = Seed(decoded)
	return nil
}
func (value Seed) MarshalJSON() ([]byte, error) { type wire Seed; return encodeStruct(wire(value)) }
func (value *EmptyJournal) UnmarshalJSON(data []byte) error {
	type wire EmptyJournal
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = EmptyJournal(decoded)
	return nil
}
func (value EmptyJournal) MarshalJSON() ([]byte, error) {
	type wire EmptyJournal
	return encodeStruct(wire(value))
}
func (value *Notice) UnmarshalJSON(data []byte) error {
	type wire Notice
	var decoded wire
	if err := decodeStruct(data, &decoded, false); err != nil {
		return err
	}
	*value = Notice(decoded)
	return nil
}
func (value Notice) MarshalJSON() ([]byte, error) { type wire Notice; return encodeStruct(wire(value)) }
func (value *Date) UnmarshalJSON(data []byte) error {
	type wire Date
	var decoded wire
	if err := decodeStruct(data, &decoded, false); err != nil {
		return err
	}
	*value = Date(decoded)
	return nil
}
