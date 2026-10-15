package mcp

import (
	"bytes"
	"encoding/json"
	"reflect"
	"slices"
	"testing"
)

// normalizeAllInputKeys preserves the original sorting of supplied fields as a compatibility reference.
func normalizeAllInputKeys(fields map[string]any, schema toolSchema) error {
	keys := make([]string, 0, len(fields))
	for key := range fields {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	for _, key := range keys {
		property, isKnown := schema.Properties[key]
		if !isKnown {
			continue
		}
		value := fields[key]
		if value == nil && !slices.Contains(schema.Required, key) {
			continue
		}
		converted, err := normalizeToolValue(value, property.kind, property.AnyOf)
		if err != nil {
			return err
		}
		fields[key] = converted
	}
	return nil
}

// FuzzInputKeyInventory compares error order and partial values across both key-inventory paths.
func FuzzInputKeyInventory(f *testing.F) {
	schemas := []toolSchema{declaredInputSchema(f, "search_articles"), declaredInputSchema(f, "get_article"), declaredInputSchema(f, "get_weekly_updates")}
	for _, text := range []string{
		`{}`, `null`, `{"article_id":"42","db":"fixture.sqlite","unknown":[{"nested":true}]}`,
		`{"limit":"bad","db":1,"0":[false],"year":true}`, `{"open_access":null,"q":null,"unknown":[]}`,
		`{"limit":2,"offset":1.0,"year":2026}`, unknownFieldPayload(f, 1000),
	} {
		for index := range schemas {
			f.Add(text, uint8(index))
		}
	}
	f.Fuzz(func(t *testing.T, text string, index uint8) {
		if len(text) > 32768 {
			t.Skip()
		}
		var actual, expected map[string]any
		for _, target := range []*map[string]any{&actual, &expected} {
			decoder := json.NewDecoder(bytes.NewBufferString(text))
			decoder.UseNumber()
			if err := decoder.Decode(target); err != nil {
				return
			}
		}
		schema := schemas[int(index)%len(schemas)]
		expectedError := normalizeAllInputKeys(expected, schema)
		actualError := normalizeToolFields(actual, schema)
		if !reflect.DeepEqual(actual, expected) || !reflect.DeepEqual(actualError, expectedError) {
			t.Fatal("key inventory changed normalization or first error", actualError, expectedError)
		}
	})
}
