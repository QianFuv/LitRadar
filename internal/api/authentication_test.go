package api

import (
	"net/http"
	"testing"
)

func TestCredentialPrecedenceDoesNotFallBackFromMalformedAuthorization(t *testing.T) {
	for _, scenario := range []struct {
		authorization []string
		cookies       []string
		token         string
		isError       bool
	}{
		{nil, []string{"litradar_session=cookie"}, "cookie", false},
		{[]string{"Bearer explicit"}, []string{"litradar_session=cookie"}, "explicit", false},
		{[]string{"bEaReR  \t"}, []string{"litradar_session=cookie"}, "cookie", false},
		{[]string{"Bearer"}, []string{"litradar_session=cookie"}, "", true},
		{[]string{"Bearer\ttoken"}, []string{"litradar_session=cookie"}, "", true},
		{[]string{"Basic token"}, []string{"litradar_session=cookie"}, "", true},
		{[]string{"Bearer 文"}, []string{"litradar_session=cookie"}, "", true},
		{[]string{"Bearer first", "Bearer second"}, nil, "first", false},
		{nil, []string{"irrelevant=x", "litradar_session=second"}, "", false},
		{nil, []string{"litradar_session=first; litradar_session=second"}, "first", false},
		{nil, []string{"litradar_session=; litradar_session=second"}, "", false},
		{nil, []string{"litradar_session =ignored; litradar_session=%61"}, "%61", false},
	} {
		headers := http.Header{}
		for _, value := range scenario.authorization {
			headers.Add("Authorization", value)
		}
		for _, value := range scenario.cookies {
			headers.Add("Cookie", value)
		}
		token, err := resolveAuthToken(headers)
		if token != scenario.token || (err != nil) != scenario.isError {
			t.Fatalf("credential resolution: token %q, error %v", token, err)
		}
		if err != nil && (err.status != 401 || err.detail != "Invalid authorization format") {
			t.Fatal(err)
		}
	}
}

func TestCookieWirePreservesClearingAndFractionalMaxAge(t *testing.T) {
	if value := sessionCookieHeader("fixture", 102.9, 100.1, true); value != "litradar_session=fixture; Max-Age=2; Path=/; SameSite=lax; HttpOnly; Secure" {
		t.Fatal(value)
	}
	if value := sessionCookieHeader("", 0, 100, false); value != "litradar_session=; Max-Age=0; Path=/; SameSite=lax; HttpOnly" {
		t.Fatal(value)
	}
	if _, hasCookie := sessionCookie(http.Header{"Cookie": {"litradar_session="}}); !hasCookie {
		t.Fatal("empty browser cookie still requires clearing")
	}
}
