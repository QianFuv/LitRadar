//go:build go1.27

// Package jsonvalue adapts standard JSON encoding to persisted string values.
package jsonvalue

import (
	jsonv1 "encoding/json"
	"encoding/json/jsontext"
	json "encoding/json/v2"
)

// ValidJson reports whether value is a valid UTF-8 JSON value with unique object names.
func ValidJson(value string) bool { return jsontext.Value(value).IsValid() }

// EncodeJson encodes deterministic JSON while retaining the public omitempty field contract.
func EncodeJson(value any) (string, error) {
	encoded, err := json.Marshal(value, json.Deterministic(true), jsonv1.OmitEmptyWithLegacySemantics(true))
	if err != nil {
		return "", err
	}
	return string(encoded), nil
}
