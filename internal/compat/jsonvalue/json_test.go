package jsonvalue_test

import (
	"encoding/json"
	"math"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

func TestValidationCompatibilityCorpus(t *testing.T) {
	for _, value := range []string{
		`null`, `true`, `[]`, `{}`, " \n null \t", `"\ufffd"`, `"\ud83d\ude00"`, `"\uD83d\uDe00"`, `"\\ud800"`,
		strings.Repeat("[", 127) + "0" + strings.Repeat("]", 127),
		`9007199254740993`, `18446744073709551615`, `-0`, `1e15`, `{"a":1,"a":2}`, `"\/"`, `"\u2028\u2029"`, "\"\u2028\u2029\"",
	} {
		if !jsonvalue.ValidJson(value) {
			t.Errorf("valid input rejected: %q", value)
		}
	}
	for _, value := range []string{
		``, ` `, `{} {}`, `null trailing`, string([]byte{'"', 0xff, '"'}), `"\ud800"`, `"\udc00"`, `"\ud800x"`, `"\ud800\u0041"`,
		strings.Repeat("[", 128) + "0" + strings.Repeat("]", 128),
		`1e400`, `-1e400`, `NaN`, `Infinity`, `+1`, `01`, "\"line\nfeed\"", "\"\x00\"",
	} {
		if jsonvalue.ValidJson(value) {
			t.Errorf("invalid input accepted: %q", value)
		}
	}
}

func TestEncodingCompatibilityCorpus(t *testing.T) {
	for _, scenario := range []struct {
		value any
		want  string
	}{
		{map[string]any{"z": 1, "a": 2}, `{"a":2,"z":1}`},
		{"<>&/", `"<>&/"`},
		{"\u2028\u2029", "\"\u2028\u2029\""},
		{`\u2028`, `"\\u2028"`},
		{"\\\u2028", "\"\\\\\u2028\""},
		{"\\\\\u2029", "\"\\\\\\\\\u2029\""},
		{json.Number("-0"), "-0"},
		{json.Number("1e15"), "1e15"},
		{json.Number("1e400"), "1e400"},
		{string([]byte{0xff}), "\"\ufffd\""},
		{json.RawMessage(`{"a":1,"a":2}`), `{"a":1,"a":2}`},
	} {
		actual, err := jsonvalue.EncodeJson(scenario.value)
		if err != nil || actual != scenario.want {
			t.Errorf("value=%#v got=%q want=%q error=%v", scenario.value, actual, scenario.want, err)
		}
	}
	for _, scenario := range []struct {
		value any
		want  string
	}{
		{math.NaN(), "json: unsupported value: NaN"},
		{math.Inf(1), "json: unsupported value: +Inf"},
		{make(chan int), "json: unsupported type: chan int"},
		{json.Number("wat"), `json: error calling MarshalJSONTo for type *json.Number: cannot parse "wat" as JSON number: invalid syntax`},
	} {
		actual, err := jsonvalue.EncodeJson(scenario.value)
		if actual != "" || err == nil || err.Error() != scenario.want {
			t.Errorf("value=%v result=%q error=%v want=%q", scenario.value, actual, err, scenario.want)
		}
	}
}
