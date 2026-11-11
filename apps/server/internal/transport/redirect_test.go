package transport

import (
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
)

type roundTripperFunc func(*http.Request) (*http.Response, error)

func (call roundTripperFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return call(request)
}

type observedBody struct {
	io.Reader
	isClosed bool
}

func (body *observedBody) Close() error { body.isClosed = true; return nil }

// TestRedirectsProtectCrossOriginCredentials checks Go's method and sensitive-header policy.
func TestRedirectsProtectCrossOriginCredentials(t *testing.T) {
	calls := 0
	wire := roundTripperFunc(func(request *http.Request) (*http.Response, error) {
		calls++
		if calls == 1 {
			return &http.Response{StatusCode: 302, Header: http.Header{"Location": {"https://other.test/next"}}, Body: io.NopCloser(strings.NewReader(""))}, nil
		}
		if request.Method != http.MethodGet || request.Header.Get("Authorization") != "" || request.Header.Get("Cookie") != "" {
			t.Error("cross-origin redirect exposed credentials or retained POST", request.Method)
		}
		return &http.Response{StatusCode: 200, Header: http.Header{}, Body: io.NopCloser(strings.NewReader("ok"))}, nil
	})
	response, err := RedirectSequence(t.Context(), wire, nil, "POST", "https://source.test/start", "body", http.Header{"Authorization": {"Bearer fixture"}, "Cookie": {"session=fixture"}}, func(next string, hops int) error {
		if next != "https://other.test/next" || hops != 1 {
			t.Error("redirect policy missed target or hop")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := response.Body.Close(); err != nil {
		t.Fatal(err)
	}
	if calls != 2 {
		t.Fatal("redirect not completed", calls)
	}
}

// TestRedirectPolicyRejectsBeforeTransmission verifies admission and response cleanup.
func TestRedirectPolicyRejectsBeforeTransmission(t *testing.T) {
	calls := 0
	body := &observedBody{Reader: strings.NewReader("discard")}
	wire := roundTripperFunc(func(*http.Request) (*http.Response, error) {
		calls++
		return &http.Response{StatusCode: 302, Header: http.Header{"Location": {"https://other.test/next"}}, Body: body}, nil
	})
	denied := errors.New("target denied")
	response, err := RedirectSequence(t.Context(), wire, nil, "GET", "https://source.test/start", "", http.Header{}, func(string, int) error { return denied })
	if response != nil || !errors.Is(err, denied) || calls != 1 || !body.isClosed {
		t.Fatalf("policy bypass or leaked response: calls=%d closed=%v err=%v", calls, body.isClosed, err)
	}
}
