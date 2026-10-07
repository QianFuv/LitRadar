package mcp

import (
	"encoding/json"
	"reflect"
	"testing"
)

func inputSchemaFixture(t *testing.T, encoded string) toolSchema {
	t.Helper()
	var schema toolSchema
	if err := json.Unmarshal([]byte(encoded), &schema); err != nil {
		t.Fatal(err)
	}
	return schema
}

func TestInputDecodingRetainsTypedValuesAndCompatibilityForms(t *testing.T) {
	schema := inputSchemaFixture(t, `{"properties":{"identifier":{"type":"integer"},"text":{"type":["string","null"]},"labels":{"anyOf":[{},{}]},"names":{"type":"array"}},"required":["identifier"]}`)
	for _, scenario := range []struct {
		name, encoded string
		identifier    int64
	}{
		{"large signed", `{"identifier":9007199254740993,"text":null,"labels":[],"names":[],"unknown":true}`, 9007199254740993},
		{"last duplicate", `{"identifier":"bad","identifier":2}`, 2},
		{"trailing ignored", `{"identifier":3} trailing`, 3},
		{"negative", `{"identifier":-9223372036854775808}`, -9223372036854775808},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			input, err := decodeInput(json.RawMessage(scenario.encoded), schema)
			if err != nil || input.fields["identifier"] != scenario.identifier {
				t.Fatal(input, err)
			}
		})
	}
}

func TestInputDecodingKeepsSortedTypeErrorsBeforeRequiredFields(t *testing.T) {
	schema := inputSchemaFixture(t, `{"properties":{"a":{"type":"integer"},"b":{"type":"boolean"},"c":{"anyOf":[{},{}]},"d":{"type":"array"},"e":{"type":"string"}},"required":["missing","a"]}`)
	for _, scenario := range []struct{ encoded, detail string }{
		{`{"b":1,"a":"bad"}`, `invalid type: string "bad", expected i64`},
		{`{"a":9223372036854775808}`, "invalid value: integer `9223372036854775808`, expected i64"},
		{`{"a":1.0}`, "invalid type: floating point `1.0`, expected i64"},
		{`{"a":null}`, "invalid type: null, expected i64"},
		{`{"a":1,"c":["x",2]}`, "data did not match any variant of untagged enum StringOrStrings"},
		{`{"a":1,"d":[true]}`, "invalid type: boolean `true`, expected a string"},
		{`{"a":1,"e":false}`, "invalid type: boolean `false`, expected a string"},
		{`{"a":1,"b":null,"c":null,"d":null,"e":null}`, "missing field `missing`"},
		{`null`, "missing field `missing`"},
		{``, "missing field `missing`"},
	} {
		input, err := decodeInput(json.RawMessage(scenario.encoded), schema)
		if input != nil || err == nil || err.Error() != scenario.detail {
			t.Fatal(scenario.encoded, input, err)
		}
	}
}

func TestArticleInputRetainsFirstFailureAndPartialParameters(t *testing.T) {
	input := &toolInput{fields: map[string]any{"journal_id": []any{"bad", "2"}, "area": []any{" x ", ""}, "search_mode": "ADVANCED", "limit": int64(0), "db": " "}}
	database, params := input.articles()
	if input.err == nil || input.err.Error() != "journal_id must be a positive integer" || database == nil || *database != "" {
		t.Fatal(input.err, database)
	}
	if !reflect.DeepEqual(params.JournalId, []int64{0, 2}) || !reflect.DeepEqual(params.Area, []string{"x", ""}) || params.SearchMode != "advanced" || params.Limit != 0 {
		t.Fatal(params)
	}
}
