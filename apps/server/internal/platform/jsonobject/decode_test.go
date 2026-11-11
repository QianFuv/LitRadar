//go:build go1.27

package jsonobject

import "testing"

func TestRequiredFieldsAndDefaults(t *testing.T) {
	type payload struct {
		Count   int64   `json:"count"`
		Enabled bool    `json:"enabled" default:"true"`
		Name    *string `json:"name"`
	}
	for _, raw := range []string{`{}`, `{"count":null}`, `[0,false,null]`, `{"count":0,"extra":1}`, `{"count":0,"count":1}`} {
		var value payload
		if err := Decode([]byte(raw), &value); err == nil {
			t.Fatal("invalid object admitted", raw)
		}
	}
	var value payload
	if err := Decode([]byte(`{"count":0}`), &value); err != nil || value.Count != 0 || value.Enabled || value.Name != nil {
		t.Fatal(value, err)
	}
}
