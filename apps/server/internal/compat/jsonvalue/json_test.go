package jsonvalue_test

import (
	"math"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
)

func TestStandardJsonEscapingAndInvalidNumbers(t *testing.T) {
	encoded, err := jsonvalue.EncodeJson("<>&\u2028")
	if err != nil || encoded != "\"<>&\u2028\"" {
		t.Fatal(encoded, err)
	}
	if encoded, err := jsonvalue.EncodeJson(math.Inf(1)); err == nil || encoded != "" {
		t.Fatal(encoded, err)
	}
	if jsonvalue.ValidJson(`{} {}`) {
		t.Fatal("multiple values accepted")
	}
}

func TestPublicOptionalZeroValuesRemainPresent(t *testing.T) {
	type response struct {
		Value *string `json:"value,omitempty"`
		Count int     `json:"count,omitempty"`
	}
	value := ""
	encoded, err := jsonvalue.EncodeJson(response{Value: &value})
	if err != nil || encoded != `{"value":""}` {
		t.Fatal(encoded, err)
	}
}
