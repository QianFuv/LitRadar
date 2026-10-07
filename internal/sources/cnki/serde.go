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

// decodeFields preserves strict retained values and positional or object field policies.
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
	var err error
	switch body[0] {
	case '[':
		err = decodeFieldSequence(body, names, hasDefaults, values)
	case '{':
		err = decodeFieldObject(body, known, denyUnknown, values)
	default:
		return nil, errFixtureJson
	}
	if err != nil {
		return nil, err
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

// decodeTyped preserves pointer nullability and allocation before nested decoding.
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
		return decodeTypedMap(raw, target)
	case reflect.Slice:
		return decodeTypedSlice(raw, target)
	}
	if json.Unmarshal(raw, target.Addr().Interface()) != nil {
		return errFixtureJson
	}
	return nil
}

// decodeFieldSequence maps admitted positional fields without requiring defaulted tails.
func decodeFieldSequence(body []byte, names []string, hasDefaults bool, values map[string]json.RawMessage) error {
	var items []json.RawMessage
	if json.Unmarshal(body, &items) != nil || len(items) > len(names) || !hasDefaults && len(items) != len(names) {
		return errFixtureJson
	}
	for index, raw := range items {
		values[names[index]] = raw
	}
	return nil
}

// decodeFieldObject validates every raw key before applying unknown and duplicate policy.
func decodeFieldObject(body []byte, known map[string]bool, denyUnknown bool, values map[string]json.RawMessage) error {
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.Token()
	for decoder.More() {
		before := decoder.InputOffset()
		key, err := decoder.Token()
		if err != nil {
			return errFixtureJson
		}
		rawKey := bytes.TrimSpace(bytes.TrimPrefix(bytes.TrimSpace(body[before:decoder.InputOffset()]), []byte(",")))
		if _, err := transport.ParseJson(rawKey); err != nil {
			return errFixtureJson
		}
		name := key.(string)
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil {
			return errFixtureJson
		}
		if !known[name] {
			if denyUnknown {
				return errFixtureJson
			}
			continue
		}
		if _, exists := values[name]; exists {
			return errFixtureJson
		}
		values[name] = raw
	}
	return nil
}

// decodeTypedMap allocates the supplied collection before decoding its entries.
func decodeTypedMap(raw []byte, target reflect.Value) error {
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
}

// decodeTypedSlice allocates the supplied collection before decoding its entries.
func decodeTypedSlice(raw []byte, target reflect.Value) error {
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
