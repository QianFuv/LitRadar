package weekly

import (
	"bytes"
	"encoding/json"
	"fmt"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

// manifestIdsReference retains the original raw-element admission and integer spelling rules.
func manifestIdsReference(value json.RawMessage) ([]int64, error) {
	ids := []int64{}
	seen := map[int64]bool{}
	if bytes.HasPrefix(bytes.TrimSpace(value), []byte("[")) {
		var values []json.RawMessage
		if json.Unmarshal(value, &values) != nil {
			return nil, ErrManifestJson
		}
		for _, value := range values {
			raw := strings.TrimSpace(string(value))
			if raw == "-0" || strings.ContainsAny(raw, ".eE") {
				continue
			}
			id, err := strconv.ParseInt(raw, 10, 64)
			if err != nil || seen[id] {
				continue
			}
			seen[id] = true
			ids = append(ids, id)
		}
	}
	return ids, nil
}

// TestManifestIdDecoderClearsReusedElements prevents ignored values from inheriting an earlier admitted integer.
func TestManifestIdDecoderClearsReusedElements(t *testing.T) {
	var values []manifestArticleId
	for _, ignored := range []string{"null", "-0", "9223372036854775808", "3.0", `"8"`, "{}", "[]"} {
		if err := json.Unmarshal([]byte("[7]"), &values); err != nil || len(values) != 1 || !values[0].isInteger || values[0].value != 7 {
			t.Fatal("integer was not admitted", values, err)
		}
		if err := json.Unmarshal([]byte("["+ignored+"]"), &values); err != nil || len(values) != 1 || values[0].isInteger || values[0].value != 0 {
			t.Fatal("ignored value inherited a previous integer", ignored, values, err)
		}
	}
}

// FuzzManifestArticleIds compares exact source ordering, duplicate removal and errors for arbitrary JSON bytes.
func FuzzManifestArticleIds(f *testing.F) {
	for _, value := range []string{"", "null", "[]", "[-0]", "[1,1,2,0,-0,-1]", "[null,3,true,\"4\",{},[5],6.0,7e0,9223372036854775808,-9223372036854775808,8]", "[1,", " [1,2] \n", "[\"\xff\",1]"} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		if len(value) > 8192 {
			t.Skip()
		}
		expected, expectedError := manifestIdsReference([]byte(value))
		actual, err := decodeManifestArticleIds([]byte(value))
		if !reflect.DeepEqual(actual, expected) || (err == nil) != (expectedError == nil) {
			t.Fatal("manifest integer admission changed", value, actual, expected, err, expectedError)
		}
	})
}

// BenchmarkManifestArticleIds measures ordinary integer lists and compatibility fallback elements.
func BenchmarkManifestArticleIds(b *testing.B) {
	for _, count := range []int{10, 10000, 100000} {
		ids := make([]int64, count)
		for index := range ids {
			ids[index] = int64(index)
		}
		body, err := json.Marshal(ids)
		if err != nil {
			b.Fatal(err)
		}
		for _, isMixed := range []bool{false, true} {
			value := body
			if isMixed {
				value = append(append([]byte(nil), body[:len(body)-1]...), []byte(",-0,null,3.0,\"4\"]")...)
			}
			b.Run(fmt.Sprintf("count=%d/mixed=%t", count, isMixed), func(b *testing.B) {
				b.ReportAllocs()
				for b.Loop() {
					actual, err := decodeManifestArticleIds(value)
					if err != nil || len(actual) != count || actual[0] != 0 || actual[count-1] != int64(count-1) {
						b.Fatal("manifest IDs changed", len(actual), err)
					}
				}
			})
		}
	}
}
