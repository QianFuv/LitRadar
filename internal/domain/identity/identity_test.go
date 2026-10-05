package identity

import (
	"encoding/json"

	"testing"
)

func TestIdJsonPreservesPrecisionAndRejectsNonIntegers(t *testing.T) {
	for _, decimal := range []string{"-9223372036854775808", "-1", "0", "9007199254740993", "9223372036854775807"} {
		for _, input := range []string{decimal, `"` + decimal + `"`} {
			var value Id
			if err := json.Unmarshal([]byte(input), &value); err != nil {
				t.Fatalf("decode %s: %v", input, err)
			}
			encoded, err := json.Marshal(value)
			if err != nil || string(encoded) != `"`+decimal+`"` {
				t.Fatalf("precision lost for %s: %s %v", input, encoded, err)
			}
		}
	}
	for _, invalid := range []string{`null`, `true`, `1.0`, `1e3`, `"9223372036854775808"`, `9223372036854775808`, `-9223372036854775809`, `[]`, `{}`, `""`} {
		var value Id
		if err := json.Unmarshal([]byte(invalid), &value); err == nil {
			t.Errorf("Accepted invalid ID %s", invalid)
		}
	}
}

func TestStablePreservesNumericIdsAndSeparatesNamespaces(t *testing.T) {
	for _, item := range []struct {
		input string
		want  Id
	}{{"-9223372036854775808", -9223372036854775808}, {"0", 0}, {"0012", 12}, {"9223372036854775807", 9223372036854775807}} {
		for _, prefix := range []string{"article", "journal"} {
			if actual := Stable(item.input, prefix); actual != item.want {
				t.Fatalf("numeric ID %s changed: %d", item.input, actual)
			}
		}
	}
	for _, input := range []string{"", "external-id", "中文标识符", "9223372036854775808"} {
		article := Stable(input, "article")
		journal := Stable(input, "journal")
		if article <= 0 || journal <= 0 || article == journal || article != Stable(input, "article") {
			t.Fatalf("unstable or unscoped identity for %q: %d %d", input, article, journal)
		}
	}
}
