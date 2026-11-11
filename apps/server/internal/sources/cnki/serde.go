//go:build go1.27

package cnki

import (
	"bytes"
	jsonv2 "encoding/json/v2"
	"errors"
)

var errFixtureJson = errors.New("invalid domestic CNKI fixture JSON; expected an object")

// UnmarshalJSON decodes object fixtures using standard typed JSON decoding.
func (data *FixtureData) UnmarshalJSON(body []byte) error {
	body = bytes.TrimSpace(body)
	if len(body) == 0 || body[0] != '{' {
		return errFixtureJson
	}
	type wire FixtureData
	var decoded wire
	if jsonv2.Unmarshal(body, &decoded) != nil {
		return errFixtureJson
	}
	*data = FixtureData(decoded)
	return nil
}
