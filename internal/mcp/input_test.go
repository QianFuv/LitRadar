package mcp

import (
	"encoding/json"
	"testing"
)

// TestDeclaredInputSchemasAcceptNullableAndStringOrStringsFields checks supported tool inputs.
func TestDeclaredInputSchemasAcceptNullableAndStringOrStringsFields(t *testing.T) {
	for _, raw := range []string{
		`{"db":null,"area":"Research","journal_id":["1","2"]}`,
		`{"db":"fixture.sqlite","area":[],"journal_id":"1"}`,
	} {
		if _, err := decodeInput(json.RawMessage(raw), declaredInputSchema(t, "search_articles")); err != nil {
			t.Fatal(raw, err)
		}
	}
}

func TestInputDecodingRetainsTypedValuesAndCompatibilityForms(t *testing.T) {
	schema := declaredInputSchema(t, "add_favorite")
	for _, scenario := range []struct {
		name, encoded string
		identifier    int64
	}{
		{"large signed", `{"article_id":"1","folder_id":9007199254740993,"db_name":null,"unknown":true}`, 9007199254740993},
		{"last duplicate", `{"article_id":"1","folder_id":"bad","folder_id":2}`, 2},
		{"negative", `{"article_id":"1","folder_id":-9223372036854775808}`, -9223372036854775808},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			input, err := decodeInput(json.RawMessage(scenario.encoded), schema)
			if err != nil || input.fields["folder_id"] != scenario.identifier {
				t.Fatal(input, err)
			}
		})
	}
}

// TestInputDecodingKeepsSortedTypeErrorsBeforeRequiredFields checks actual tool declarations.
func TestInputDecodingKeepsSortedTypeErrorsBeforeRequiredFields(t *testing.T) {
	for _, scenario := range []struct{ tool, encoded, detail string }{
		{"add_favorite", `{"folder_id":true,"article_id":false}`, "invalid type: boolean `false`, expected a string"},
		{"add_favorite", `{"folder_id":9223372036854775808}`, "invalid value: integer `9223372036854775808`, expected i64"},
		{"add_favorite", `{"folder_id":1.0}`, "invalid type: floating point `1.0`, expected i64"},
		{"add_favorite", `{"folder_id":null}`, "invalid type: null, expected i64"},
		{"search_articles", `{"area":["x",2]}`, "data did not match any variant of untagged enum StringOrStrings"},
		{"search_articles", `{"abs_rating":[true]}`, "invalid type: boolean `true`, expected a string"},
		{"search_articles", `{"db":false}`, "invalid type: boolean `false`, expected a string"},
		{"get_article", `{"db":null}`, "missing field `article_id`"},
		{"get_article", `null`, "missing field `article_id`"},
		{"get_article", ``, "missing field `article_id`"},
	} {
		input, err := decodeInput(json.RawMessage(scenario.encoded), declaredInputSchema(t, scenario.tool))
		if input != nil || err == nil || err.Error() != scenario.detail {
			t.Fatal(scenario.encoded, input, err)
		}
	}
}

// TestUnknownInputFieldsDoNotChangeValidation checks extension fields against a real required input.
func TestUnknownInputFieldsDoNotChangeValidation(t *testing.T) {
	schema := declaredInputSchema(t, "get_article")
	input, err := decodeInput(json.RawMessage(`{"0":{"nested":["kept",true]},"article_id":"2","aa":[false],"db":null}`), schema)
	if err != nil || input.fields["article_id"] != "2" {
		t.Fatal(input, err)
	}
	for _, scenario := range []struct{ encoded, expected string }{
		{`{"0":[],"article_id":true,"aa":{},"db":1}`, "invalid type: boolean `true`, expected a string"},
		{`{"0":[],"db":1}`, "invalid type: integer `1`, expected a string"},
		{`{"0":[],"aa":{}}`, "missing field `article_id`"},
	} {
		input, err := decodeInput(json.RawMessage(scenario.encoded), schema)
		if input != nil || err == nil || err.Error() != scenario.expected {
			t.Fatal("unknown fields changed validation precedence", input, err)
		}
	}
}

func TestArticleInputRetainsFirstFailure(t *testing.T) {
	input := &toolInput{fields: map[string]any{"journal_id": []any{"bad", "2"}, "area": []any{" x ", ""}, "search_mode": "ADVANCED", "limit": int64(0), "db": " "}}
	input.articles()
	if input.err == nil || input.err.Error() != "journal_id must be a positive integer" {
		t.Fatal(input.err)
	}
}
