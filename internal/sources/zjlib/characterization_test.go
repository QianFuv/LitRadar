package zjlib

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

// TestQrCompleteAfterPollingDeadline preserves acceptance of an already admitted request.
func TestQrCompleteAfterPollingDeadline(t *testing.T) {
	live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		time.Sleep(1100 * time.Millisecond)
		fmt.Fprint(writer, `{"data":{"status":"COMPLETE","data":" "}}`)
	}), DefaultLiveConfig(), time.Time{})
	token, err := live.PollQrLogin(context.Background(), "qr", 1, 0.1)
	if err != nil || token != " " {
		t.Fatalf("admitted completion rejected after polling deadline: %q %v", token, err)
	}
}

// TestProxyHopLimitPrecedesMissingLocation preserves the redirect error admission order.
func TestProxyHopLimitPrecedesMissingLocation(t *testing.T) {
	var requests atomic.Int32
	live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		count := requests.Add(1)
		if count < 5 {
			http.Redirect(writer, request, fmt.Sprintf("/login/hop%d", count), 302)
			return
		}
		writer.WriteHeader(302)
	}), DefaultLiveConfig(), time.Time{})
	_, shouldRetry, err := live.enterProxy(context.Background(), server.URL+"/login/hop0")
	if err == nil || err.Error() != "zyproxy login exceeded 4 redirect hops." || shouldRetry || requests.Load() != 5 {
		t.Fatalf("redirect admission changed: %d %v %v", requests.Load(), shouldRetry, err)
	}
}

// TestRestoredCookiesMergeAndOmitExpiry preserves the live persisted-cookie policy.
func TestRestoredCookiesMergeAndOmitExpiry(t *testing.T) {
	live, err := NewLiveTransport(DefaultLiveConfig(), transport.Proxy{}, time.Time{})
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	live.LoadCookies([]Cookie{{Name: "keep", Value: "retained", Path: "/"}})
	expires := int64(1)
	live.LoadCookies([]Cookie{{Name: "session", Value: "first", Path: "/", Expires: &expires}, {Name: "session", Value: "last", Path: "/", Expires: &expires}})
	location, _ := url.Parse(WwwBase)
	values := map[string]string{}
	for _, cookie := range live.state.redirect.Jar.Cookies(location) {
		values[cookie.Name] = cookie.Value
	}
	if values["keep"] != "retained" || values["session"] != "last" {
		t.Fatalf("cookie merge or omitted expiry changed: %v", values)
	}
}

// TestShareCallbackErrorPrecedesDomainAdmission preserves extraction error priority.
func TestShareCallbackErrorPrecedesDomainAdmission(t *testing.T) {
	text := `var sign='s';var url='https://example.com/callback';var domainUrl='invalid';sso-login/cookie/sync`
	_, err := extractShareCookieSync(text, defaultEndpoints())
	if err == nil || !strings.Contains(err.Error(), "unexpected endpoint") {
		t.Fatalf("callback admission lost priority: %v", err)
	}
}

// TestSearchDuplicateSkipsConflictingDownload preserves deduplication before row validation.
func TestSearchDuplicateSkipsConflictingDownload(t *testing.T) {
	text := `<tr><a href="/kns55/detail/detail.aspx?FileName=A">First</a></tr><tr><a href="/kns55/detail/detail.aspx?FileName=A">Second</a><a href="https://example.com/download.aspx">Invalid hint</a></tr>`
	results, err := ParseSearchResults(text, ProxyBase+"/kns55/brief/brief.aspx")
	if err != nil || len(results) != 1 {
		t.Fatalf("duplicate download hint was inspected: %v %v", results, err)
	}
	if results[0].Title != "First" || results[0].DownloadUrl != nil {
		t.Fatal(results)
	}
}
