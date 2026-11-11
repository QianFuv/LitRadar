package transport

import (
	"context"
	"errors"
	"net/http"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ErrRequestBuild identifies a URL that cannot become an upstream HTTP request.
var ErrRequestBuild = errors.New("builder error")

// RedirectSequence delegates redirect methods, cookies and sensitive headers to net/http.
// All redirects share ctx and pass the source-specific target and hop policy before sending.
func RedirectSequence(ctx context.Context, wire http.RoundTripper, jar http.CookieJar, method, location, body string, headers http.Header, policy func(string, int) error) (*http.Response, error) {
	parsed, err := whatwg.NewParser().Parse(location)
	if err != nil || parsed.Hostname() == "" || len(parsed.Href(false)) > 65534 {
		return nil, ErrRequestBuild
	}
	request, err := http.NewRequestWithContext(ctx, method, parsed.Href(true), strings.NewReader(body))
	if err != nil {
		return nil, ErrRequestBuild
	}
	request.Header = headers.Clone()
	client := http.Client{Transport: wire, Jar: jar, CheckRedirect: func(next *http.Request, via []*http.Request) error {
		if policy == nil {
			return http.ErrUseLastResponse
		}
		if len(next.URL.String()) > 65534 {
			return ErrRequestBuild
		}
		return policy(next.URL.String(), len(via))
	}}
	response, err := client.Do(request)
	if err != nil {
		return nil, err
	}
	return response, nil
}
