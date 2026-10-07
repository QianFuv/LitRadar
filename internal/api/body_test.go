package api

import (
	"errors"
	"fmt"
	"io"
	"net/http/httptest"
	"reflect"

	"strings"
	"testing"
)

func TestJsonBodyLimitAndOptionalAbsence(t *testing.T) {
	for _, size := range []int{2 * 1024 * 1024, 2*1024*1024 + 1} {
		body := "{}" + strings.Repeat(" ", size-2)
		request := httptest.NewRequest("POST", "/", strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		_, failure := extractBody(request, requestBodies["TokenCreateRequest"], false)
		if size == 2*1024*1024 && failure != nil {
			t.Fatal(failure)
		}
		if size > 2*1024*1024 && (failure == nil || failure.status != 413) {
			t.Fatal("oversize body not rejected")
		}
		request = httptest.NewRequest("POST", "/", strings.NewReader(body))
		_, failure = extractBody(request, requestBodies["AdminInviteCodeCreate"], true)
		if failure != nil {
			t.Fatal(failure)
		}
	}
}

// TestJsonContentTypeRetainsParameterGrammar fixes suffix, duplicate and quoted parameter behavior.
func TestJsonContentTypeRetainsParameterGrammar(t *testing.T) {
	cases := []struct {
		value string
		want  bool
	}{
		{"application/json", true}, {"APPLICATION/JSON", true}, {"application/problem+json", true},
		{"application/+json", false}, {"application/json ", false}, {"application /json", false},
		{"application/json;", true}, {"application/json;   ", true}, {"application/json;x=", true},
		{"application/json;x=a;x=b", true}, {`application/json;x="a"`, true}, {`application/json;x=""`, false},
		{"application/json;\tx=a", false}, {"application/json;x=a ;y=b", false},
	}
	for _, test := range cases {
		t.Run(test.value, func(t *testing.T) {
			if actual := isJsonContentType(test.value); actual != test.want {
				t.Fatalf("accepted=%v want=%v", actual, test.want)
			}
		})
	}
}

type bodyErrorReader struct{ remaining int }

// Read supplies a fixed byte count with an error so buffering precedence can be exercised.
func (reader *bodyErrorReader) Read(buffer []byte) (int, error) {
	count := min(len(buffer), reader.remaining)
	for index := range count {
		buffer[index] = ' '
	}
	reader.remaining -= count
	if reader.remaining == 0 {
		return count, errors.New("fixture read failure")
	}
	return count, nil
}

// TestBodyHeadersAndBufferFailuresPreserveAdmissionOrder fixes first-header and buffering precedence.
func TestBodyHeadersAndBufferFailuresPreserveAdmissionOrder(t *testing.T) {
	cases := []struct {
		headers      []string
		size, status int
	}{
		{[]string{}, 0, 415}, {[]string{"text/plain", "application/json"}, 0, 415},
		{[]string{"application/json", "text/plain"}, 10, 400},
		{[]string{"application/json"}, 2*1024*1024 + 1, 413},
	}
	for _, test := range cases {
		request := httptest.NewRequest("POST", "/", nil)
		request.Header["Content-Type"] = test.headers
		request.Body = io.NopCloser(&bodyErrorReader{remaining: test.size})
		_, failure := extractBody(request, requestBodies["TokenCreateRequest"], true)
		if failure == nil || failure.status != test.status || !failure.isPlain {
			t.Fatalf("failure=%+v", failure)
		}
	}
}

// TestBodyRejectionPreservesDataPrecedenceAndPositions fixes early key rejection semantics.
func TestBodyRejectionPreservesDataPrecedenceAndPositions(t *testing.T) {
	cases := []struct {
		kind, body, path, detail string
		position                 int
		isData                   bool
	}{
		{"LoginRequest", `{}`, "", "missing field `username` at line 1 column 2", 2, true},
		{"LoginRequest", `{"username":"a","username"}`, "", "duplicate field `username` at line 1 column 27", 27, true},
		{"AdminInviteCodeCreate", `{"unknown"}`, "unknown", "unknown field `unknown`, expected `expires_at` or `max_uses` at line 1 column 11", 11, true},
		{"TokenCreateRequest", `{"ttl":null}`, "ttl", "invalid type: null, expected i64 at line 1 column 11", 11, true},
		{"TokenCreateRequest", `{"ttl":1,}`, "", "trailing comma at line 1 column 10", 9, false},
		{"LoginRequest", `["a"]`, "", "invalid length 1, expected struct LoginRequest with 2 elements at line 1 column 5", 5, true},
	}
	for _, test := range cases {
		t.Run(test.kind+test.body, func(t *testing.T) {
			scanner := bodyScanner{body: []byte(test.body)}
			_, err := scanner.typed(requestBodies[test.kind], "", 0)
			if err == nil {
				t.Fatal("invalid body accepted")
			}
			failure := err.(*bodyFailure)
			if failure.detail != test.detail || failure.path != test.path || failure.isData != test.isData || scanner.position != test.position {
				t.Fatalf("failure=%+v position=%d", failure, scanner.position)
			}
		})
	}
}

// TestBodyDefaultsAndDictionaryDuplicatesPreserveTypedValues fixes object and sequence defaults.
func TestBodyDefaultsAndDictionaryDuplicatesPreserveTypedValues(t *testing.T) {
	cases := []struct {
		kind, body string
		want       map[string]any
	}{
		{"TokenCreateRequest", `{}`, map[string]any{"name": "", "ttl": int64(604800)}},
		{"TokenCreateRequest", `[]`, map[string]any{"name": "", "ttl": int64(604800)}},
		{"AdminInviteCodeCreate", `{}`, map[string]any{"expires_at": nil, "max_uses": nil}},
		{"RuntimeSettingsUpdate", `{"values":{"log_format":"json","log_format":"compact"}}`, map[string]any{"values": map[string]any{"log_format": "compact"}, "secret_pool_updates": map[string]any{}}},
	}
	for _, test := range cases {
		t.Run(test.kind+test.body, func(t *testing.T) {
			scanner := bodyScanner{body: []byte(test.body)}
			value, err := scanner.typed(requestBodies[test.kind], "", 0)
			if err != nil || !reflect.DeepEqual(value, test.want) {
				t.Fatalf("value=%#v error=%v", value, err)
			}
		})
	}
	scanner := bodyScanner{body: []byte(`{"values":{"x":false,"x":"valid"}}`)}
	if _, err := scanner.typed(requestBodies["RuntimeSettingsUpdate"], "", 0); err == nil {
		t.Fatal("invalid earlier dictionary value was hidden by a duplicate")
	}
}

// TestBodyTypedDepthLimitDoesNotLimitIgnoredUnknownValues fixes the two recursion policies.
func TestBodyTypedDepthLimitDoesNotLimitIgnoredUnknownValues(t *testing.T) {
	for _, depth := range []int{127, 128} {
		t.Run(fmt.Sprint(depth), func(t *testing.T) {
			kind := stringBody
			for range depth {
				kind = listBody(kind)
			}
			input := strings.Repeat("[", depth) + `"value"` + strings.Repeat("]", depth)
			scanner := bodyScanner{body: []byte(input)}
			_, err := scanner.typed(kind, "", 0)
			if (err != nil) != (depth == 128) {
				t.Fatalf("typed depth=%d error=%v", depth, err)
			}
		})
	}
	input := `{"unknown":` + strings.Repeat("[", 200) + `"\ud800"` + strings.Repeat("]", 200) + `}`
	scanner := bodyScanner{body: []byte(input)}
	if _, err := scanner.typed(requestBodies["TokenCreateRequest"], "", 0); err != nil {
		t.Fatal(err)
	}
}
