//go:build go1.27

// Package jsonobject validates required object fields before typed standard JSON decoding.
package jsonobject

import (
	"bytes"
	"encoding/json/jsontext"
	json "encoding/json/v2"
	"errors"
	"reflect"
	"strings"
)

// Options preserves optional tails and permitted extra fields at specific protocol boundaries.
type Options struct {
	// AllowUnknown permits additional object members.
	AllowUnknown bool
	// OptionalTail permits omitted fields at the end of a worker bootstrap struct.
	OptionalTail int
}

// Decode rejects missing or null required fields and unknown fields in a typed object.
// Pointer fields are optional. Values are decoded by encoding/json.
func Decode(data []byte, target any) error {
	return DecodeWithOptions(data, target, Options{})
}

// DecodeWithOptions validates required fields and applies standard typed decoding.
func DecodeWithOptions(data []byte, target any, options Options) error {
	var fields map[string]jsontext.Value
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	if fields == nil {
		return errors.New("expected a JSON object")
	}
	kind := reflect.TypeOf(target).Elem()
	for index := range kind.NumField() {
		field := kind.Field(index)
		name, _, _ := strings.Cut(field.Tag.Get("json"), ",")
		if field.Type.Kind() != reflect.Pointer {
			raw, exists := fields[name]
			if !exists && (field.Tag.Get("default") == "true" || index >= kind.NumField()-options.OptionalTail) {
				continue
			}
			if !exists || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
				return errors.New("missing or null required JSON field: " + name)
			}
		}
	}
	return json.Unmarshal(data, target, json.RejectUnknownMembers(!options.AllowUnknown))
}
