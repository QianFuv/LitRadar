package cnki

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"

	"github.com/QianFuv/LitRadar/internal/transport"
)

var errFixtureJson = errors.New("invalid domestic CNKI fixture JSON")

// UnmarshalJSON preserves defaulted positional fixtures and rejects invalid present field types.
func (data *FixtureData) UnmarshalJSON(body []byte) error {
	fields := []string{"journal_search_html", "journal_detail_html", "year_issues_html", "issue_article_pages", "article_detail_html", "article_detail_status_codes", "fail_endpoint"}
	values, err := decodeFields(body, fields, true, false)
	if err != nil {
		return err
	}
	type plain FixtureData
	var decoded plain
	value := reflect.ValueOf(&decoded).Elem()
	for index, key := range fields {
		if raw, exists := values[key]; exists {
			if err := decodeTyped(raw, value.Field(index)); err != nil {
				return err
			}
		}
	}
	*data = FixtureData(decoded)
	return nil
}
func decodeFields(body []byte, names []string, hasDefaults, denyUnknown bool) (map[string]json.RawMessage, error) {
	if !json.Valid(body) {
		return nil, errFixtureJson
	}
	body = bytes.TrimSpace(body)
	values := map[string]json.RawMessage{}
	known := map[string]bool{}
	for _, name := range names {
		known[name] = true
	}
	if body[0] == '[' {
		var items []json.RawMessage
		if json.Unmarshal(body, &items) != nil || len(items) > len(names) || !hasDefaults && len(items) != len(names) {
			return nil, errFixtureJson
		}
		for index, raw := range items {
			values[names[index]] = raw
		}
	} else if body[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(body))
		decoder.Token()
		for decoder.More() {
			before := decoder.InputOffset()
			key, err := decoder.Token()
			if err != nil {
				return nil, errFixtureJson
			}
			rawKey := bytes.TrimSpace(bytes.TrimPrefix(bytes.TrimSpace(body[before:decoder.InputOffset()]), []byte(",")))
			if _, err := transport.ParseJson(rawKey); err != nil {
				return nil, errFixtureJson
			}
			name := key.(string)
			var raw json.RawMessage
			if decoder.Decode(&raw) != nil {
				return nil, errFixtureJson
			}
			if !known[name] {
				if denyUnknown {
					return nil, errFixtureJson
				}
				continue
			}
			if _, exists := values[name]; exists {
				return nil, errFixtureJson
			}
			values[name] = raw
		}
	} else {
		return nil, errFixtureJson
	}
	for _, raw := range values {
		nested := append([]byte{'['}, raw...)
		nested = append(nested, ']')
		if _, err := transport.ParseJson(nested); err != nil {
			return nil, errFixtureJson
		}
	}
	return values, nil
}
func decodeTyped(raw []byte, target reflect.Value) error {
	raw = bytes.TrimSpace(raw)
	if bytes.Equal(raw, []byte("null")) {
		if target.Kind() != reflect.Pointer {
			return errFixtureJson
		}
		target.SetZero()
		return nil
	}
	if target.Kind() == reflect.Pointer {
		target.Set(reflect.New(target.Type().Elem()))
		return decodeTyped(raw, target.Elem())
	}
	switch target.Kind() {
	case reflect.Map:
		var values map[string]json.RawMessage
		if json.Unmarshal(raw, &values) != nil {
			return errFixtureJson
		}
		target.Set(reflect.MakeMapWithSize(target.Type(), len(values)))
		for key, item := range values {
			value := reflect.New(target.Type().Elem()).Elem()
			if err := decodeTyped(item, value); err != nil {
				return err
			}
			target.SetMapIndex(reflect.ValueOf(key), value)
		}
		return nil
	case reflect.Slice:
		var values []json.RawMessage
		if json.Unmarshal(raw, &values) != nil {
			return errFixtureJson
		}
		target.Set(reflect.MakeSlice(target.Type(), len(values), len(values)))
		for index, item := range values {
			if err := decodeTyped(item, target.Index(index)); err != nil {
				return err
			}
		}
		return nil
	}
	if json.Unmarshal(raw, target.Addr().Interface()) != nil {
		return errFixtureJson
	}
	return nil
}
