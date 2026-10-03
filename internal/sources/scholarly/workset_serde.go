package scholarly

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
)

var errWorksetJson = errors.New("invalid Crossref workset metadata")

func decodeWorksetStruct(body []byte, target any) error {
	if _, err := transport.ParseJson(body); err != nil {
		return errWorksetJson
	}
	value := reflect.ValueOf(target).Elem()
	kind := value.Type()
	names := map[string]int{}
	seen := make([]bool, kind.NumField())
	for index := 0; index < kind.NumField(); index++ {
		name, _, _ := strings.Cut(kind.Field(index).Tag.Get("json"), ",")
		names[name] = index
	}
	body = bytes.TrimSpace(body)
	if body[0] == '[' {
		var fields []json.RawMessage
		if json.Unmarshal(body, &fields) != nil || len(fields) != kind.NumField() {
			return errWorksetJson
		}
		for index, raw := range fields {
			if err := decodeWorksetValue(raw, value.Field(index)); err != nil {
				return err
			}
			seen[index] = true
		}
	} else if body[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(body))
		decoder.Token()
		for decoder.More() {
			name, err := decoder.Token()
			if err != nil {
				return errWorksetJson
			}
			index, ok := names[name.(string)]
			if !ok || seen[index] {
				return errWorksetJson
			}
			seen[index] = true
			var raw json.RawMessage
			if decoder.Decode(&raw) != nil {
				return errWorksetJson
			}
			if err := decodeWorksetValue(raw, value.Field(index)); err != nil {
				return err
			}
		}
	} else {
		return errWorksetJson
	}
	for index, hasField := range seen {
		if !hasField && value.Field(index).Kind() != reflect.Pointer {
			return errWorksetJson
		}
	}
	return nil
}
func decodeWorksetValue(raw []byte, value reflect.Value) error {
	raw = bytes.TrimSpace(raw)
	if value.Kind() == reflect.Pointer {
		if bytes.Equal(raw, []byte("null")) {
			value.SetZero()
			return nil
		}
		value.Set(reflect.New(value.Type().Elem()))
		return decodeWorksetValue(raw, value.Elem())
	}
	if bytes.Equal(raw, []byte("null")) {
		return errWorksetJson
	}
	if value.Kind() == reflect.Int64 {
		number, err := domain.ParseNumber(json.Number(raw))
		if err != nil {
			return errWorksetJson
		}
		integer, ok := number.AsInt64()
		if !ok {
			return errWorksetJson
		}
		value.SetInt(integer)
		return nil
	}
	if json.Unmarshal(raw, value.Addr().Interface()) != nil {
		return errWorksetJson
	}
	return nil
}
func worksetKind(body []byte) (string, error) {
	if _, err := transport.ParseJson(body); err != nil {
		return "", errWorksetJson
	}
	body = bytes.TrimSpace(body)
	if len(body) == 0 {
		return "", errWorksetJson
	}
	var kind string
	if body[0] == '[' {
		var fields []json.RawMessage
		if json.Unmarshal(body, &fields) != nil || len(fields) == 0 || json.Unmarshal(fields[0], &kind) != nil {
			return "", errWorksetJson
		}
		return kind, nil
	}
	if body[0] != '{' {
		return "", errWorksetJson
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.Token()
	hasKind := false
	for decoder.More() {
		name, _ := decoder.Token()
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil {
			return "", errWorksetJson
		}
		if name == "kind" {
			if hasKind || json.Unmarshal(raw, &kind) != nil {
				return "", errWorksetJson
			}
			hasKind = true
		}
	}
	if !hasKind {
		return "", errWorksetJson
	}
	return kind, nil
}

func decodeUnitPhase(body []byte) error {
	body = bytes.TrimSpace(body)
	if body[0] == '[' {
		var fields []json.RawMessage
		if json.Unmarshal(body, &fields) != nil || len(fields) != 1 {
			return errWorksetJson
		}
	}
	return nil
}

func encodeWorksetStruct(value any) ([]byte, error) {
	fields := reflect.ValueOf(value)
	kind := fields.Type()
	var output bytes.Buffer
	output.WriteByte('{')
	count := 0
	for index := 0; index < fields.NumField(); index++ {
		name, options, _ := strings.Cut(kind.Field(index).Tag.Get("json"), ",")
		field := fields.Field(index)
		if options == "omitempty" && field.Kind() == reflect.Pointer && field.IsNil() {
			continue
		}
		encoded, err := encodeWorksetValue(field)
		if err != nil {
			return nil, err
		}
		key, err := domain.Json(name)
		if err != nil {
			return nil, err
		}
		if count > 0 {
			output.WriteByte(',')
		}
		count++
		output.Write(key)
		output.WriteByte(':')
		output.Write(encoded)
	}
	output.WriteByte('}')
	return output.Bytes(), nil
}
func encodeWorksetValue(value reflect.Value) ([]byte, error) {
	if value.Kind() == reflect.Pointer {
		if value.IsNil() {
			return []byte("null"), nil
		}
		return encodeWorksetValue(value.Elem())
	}
	if marshaler, ok := value.Interface().(json.Marshaler); ok {
		return marshaler.MarshalJSON()
	}
	switch value.Kind() {
	case reflect.Uint8, reflect.Uint32, reflect.Uint64:
		return domain.Json(value.Uint())
	case reflect.Int64:
		return domain.Json(value.Int())
	case reflect.String:
		return domain.Json(value.String())
	case reflect.Bool:
		return domain.Json(value.Bool())
	case reflect.Slice:
		var output bytes.Buffer
		output.WriteByte('[')
		for index := 0; index < value.Len(); index++ {
			if index > 0 {
				output.WriteByte(',')
			}
			encoded, err := encodeWorksetValue(value.Index(index))
			if err != nil {
				return nil, err
			}
			output.Write(encoded)
		}
		output.WriteByte(']')
		return output.Bytes(), nil
	case reflect.Struct:
		return encodeWorksetStruct(value.Interface())
	}
	return nil, errWorksetJson
}
