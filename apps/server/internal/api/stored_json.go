package api

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"unicode/utf8"
)

func storedObject(encoded string) (map[string]json.RawMessage, error) {
	if !utf8.ValidString(encoded) {
		return nil, errors.New("invalid stored JSON")
	}
	decoder := json.NewDecoder(bytes.NewBufferString(encoded))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return nil, errors.New("invalid stored JSON object")
	}
	fields := map[string]json.RawMessage{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return nil, err
		}
		name, ok := token.(string)
		if !ok {
			return nil, errors.New("invalid stored JSON field")
		}
		if _, exists := fields[name]; exists {
			return nil, errors.New("duplicate stored JSON field")
		}
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			return nil, err
		}
		fields[name] = raw
	}
	if _, err := decoder.Token(); err != nil {
		return nil, err
	}
	if _, err := decoder.Token(); !errors.Is(err, io.EOF) {
		return nil, errors.New("invalid stored JSON suffix")
	}
	return fields, nil
}

func storedFields(encoded string, names []string) (map[string]json.RawMessage, error) {
	if !utf8.ValidString(encoded) {
		return nil, errors.New("invalid stored JSON")
	}
	if len(bytes.TrimSpace([]byte(encoded))) > 0 && bytes.TrimSpace([]byte(encoded))[0] == '[' {
		var values []json.RawMessage
		if err := json.Unmarshal([]byte(encoded), &values); err != nil {
			return nil, err
		}
		if len(values) != len(names) {
			return nil, errors.New("invalid stored tuple length")
		}
		fields := map[string]json.RawMessage{}
		for index, name := range names {
			fields[name] = values[index]
		}
		return fields, nil
	}
	return storedObject(encoded)
}
