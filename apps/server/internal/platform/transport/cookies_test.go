package transport

import (
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"testing"
	"time"
)

func TestPerUserCookiesAcrossRealRedirect(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path == "/login" {
			http.SetCookie(writer, &http.Cookie{Name: "session", Value: request.URL.Query().Get("user"), Path: "/private"})
			http.Redirect(writer, request, "/private/check", http.StatusTemporaryRedirect)
			return
		}
		io.WriteString(writer, request.Header.Get("Cookie"))
	}))
	defer server.Close()
	for _, user := range []string{"alice", "bob"} {
		selected, _ := New("")
		defer selected.CloseIdleConnections()
		selected.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
		}
		client := &http.Client{Transport: selected, Jar: NewCookieJar(), Timeout: time.Second}
		for _, scenario := range []struct{ path, expected string }{
			{"/private/check", ""}, {"/login?user=" + user, "session=" + user}, {"/public", ""},
		} {
			response, err := client.Get("http://session.fixture.test" + scenario.path)
			if err != nil {
				t.Fatal(err)
			}
			body, err := io.ReadAll(response.Body)
			response.Body.Close()
			if err != nil || string(body) != scenario.expected {
				t.Fatalf("user %s path %s cookie %q %v", user, scenario.path, body, err)
			}
		}
	}
}

func TestExplicitLegacyPublicSuffixSecureAndExpiryPolicy(t *testing.T) {
	jar := NewCookieJar()
	origin, _ := url.Parse("https://login.example.co.uk/private")
	jar.SetCookies(origin, []*http.Cookie{
		{Name: "legacy-domain", Value: "value", Domain: "co.uk", Path: "/"},
		{Name: "secure", Value: "value", Secure: true, Path: "/"},
		{Name: "expired", Value: "value", Expires: time.Now().Add(-time.Hour), Path: "/"},
		{Name: "foreign", Value: "value", Domain: "unrelated.invalid", Path: "/"},
	})
	if actual := jar.Cookies(origin); len(actual) != 2 {
		t.Fatalf("policy changed: %v", actual)
	}
	sibling, _ := url.Parse("http://other.co.uk/")
	if actual := jar.Cookies(sibling); len(actual) != 1 || actual[0].Name != "legacy-domain" {
		t.Fatalf("legacy public suffix policy changed: %v", actual)
	}
	origin.Scheme = "http"
	if actual := jar.Cookies(origin); len(actual) != 1 || actual[0].Name != "legacy-domain" {
		t.Fatalf("Secure cookie leaked: %v", actual)
	}
}
