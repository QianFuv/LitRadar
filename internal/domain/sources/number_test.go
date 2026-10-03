package sources

import (
	"encoding/json"
	"math"
	"os"
	"strconv"
	"testing"
)

func TestFrozenSerdeNumbers(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/number-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Observations []struct {
			Input  string
			Output struct {
				Valid            bool
				Bits             string
				Text             string
				Signed, Unsigned *string
			}
		}
	}
	if err := json.Unmarshal(body, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, observation := range fixture.Observations {
		t.Run(observation.Input, func(t *testing.T) {
			number, err := ParseNumber(json.Number(observation.Input))
			if (err == nil) != observation.Output.Valid {
				t.Fatalf("%v want valid=%v", err, observation.Output.Valid)
			}
			if err != nil {
				return
			}
			if number.String() != observation.Output.Text {
				t.Fatalf("decimal %s want %s", number.String(), observation.Output.Text)
			}
			if bits := strconv.FormatUint(math.Float64bits(number.AsFloat64()), 10); bits != observation.Output.Bits {
				t.Fatalf("float bits %s want %s", bits, observation.Output.Bits)
			}
			signed, hasSigned := number.AsInt64()
			if hasSigned != (observation.Output.Signed != nil) || hasSigned && strconv.FormatInt(signed, 10) != *observation.Output.Signed {
				t.Fatalf("signed %d/%v want %v", signed, hasSigned, observation.Output.Signed)
			}
			unsigned, hasUnsigned := number.AsUint64()
			if hasUnsigned != (observation.Output.Unsigned != nil) || hasUnsigned && strconv.FormatUint(unsigned, 10) != *observation.Output.Unsigned {
				t.Fatalf("unsigned %d/%v want %v", unsigned, hasUnsigned, observation.Output.Unsigned)
			}
		})
	}
}

func TestNumberTokenRequiresExactJsonGrammar(t *testing.T) {
	for _, input := range []string{"", "+1", "01", "1.", ".1", "NaN", "Inf", "1e", "0x1p0", "1_0", "1 ", " 1", "true", "[]"} {
		if _, err := ParseNumber(json.Number(input)); err != ErrInvalidNumber {
			t.Fatalf("%q: %v", input, err)
		}
	}
}
