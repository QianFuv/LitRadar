package sources

import (
	"encoding/json"

	"testing"
)

func TestNumberTokenRequiresExactJsonGrammar(t *testing.T) {
	for _, input := range []string{"", "+1", "01", "1.", ".1", "NaN", "Inf", "1e", "0x1p0", "1_0", "1 ", " 1", "true", "[]"} {
		if _, err := ParseNumber(json.Number(input)); err != ErrInvalidNumber {
			t.Fatalf("%q: %v", input, err)
		}
	}
}
