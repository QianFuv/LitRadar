package sources

import (
	"encoding/json"
	"math"
	"testing"
)

func TestJsonPayloadIdentity(t *testing.T) {
	value := map[string]any{"z": []any{json.Number("-0"), json.Number("18446744073709551615"), json.Number("1e-6"), json.Number("1e15")}, "a": "<>&\u2028\u2029\x00\b\f\n\r\t\"\\"}
	encoded, err := Json(value)
	if err != nil {
		t.Fatal(err)
	}
	want := "{\"a\":\"<>&\u2028\u2029\\u0000\\b\\f\\n\\r\\t\\\"\\\\\",\"z\":[-0.0,18446744073709551615,1e-6,1000000000000000.0]}"
	if string(encoded) != want {
		t.Fatalf("%s want %s", encoded, want)
	}
	for _, invalid := range []any{math.NaN(), math.Inf(1), "\xff", json.Number("1e400"), make(chan int)} {
		if _, err := Json(invalid); err == nil {
			t.Fatalf("accepted invalid %T", invalid)
		}
	}
}
