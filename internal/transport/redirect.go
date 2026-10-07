package transport

import (
	"context"
	"errors"
	"net/http"
	"net/url"
	"strings"
	"unicode/utf8"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ErrRequestBuild identifies a URL that cannot become an upstream HTTP request.
var ErrRequestBuild = errors.New("builder error")

func redirectLocation(base, reference string) (string, bool) {
	if !utf8.ValidString(reference) {
		return "", false
	}
	location, err := whatwg.NewParser().ParseRef(base, reference)
	if err != nil || location.Hostname() == "" || len(location.Href(false)) > 65534 {
		return "", false
	}
	return location.Href(true), true
}

func wireRequest(ctx context.Context, method, location, body string, headers http.Header) (*http.Request, error) {
	parsed, err := whatwg.NewParser().Parse(location)
	if err != nil || parsed.Hostname() == "" || len(parsed.Href(false)) > 65534 {
		return nil, ErrRequestBuild
	}
	request, err := http.NewRequestWithContext(ctx, method, "http://request.invalid/", strings.NewReader(body))
	if err != nil {
		return nil, ErrRequestBuild
	}
	path, err := url.PathUnescape(parsed.Pathname())
	if err != nil {
		path = parsed.Pathname()
	}
	request.URL = &url.URL{Scheme: parsed.Scheme(), Host: parsed.Host(), Path: path, Opaque: "//" + parsed.Host() + parsed.Pathname(), RawQuery: strings.TrimPrefix(parsed.Search(), "?"), ForceQuery: strings.Contains(parsed.Href(true), "?") && parsed.Search() == ""}
	request.Host = parsed.Host()
	request.Header = headers.Clone()
	return request, nil
}

// RedirectSequence normalizes Location before applying source policy, preserving
// malformed Location responses and one shared context for the entire sequence.
// A nil policy returns the first response without examining Location.
func RedirectSequence(ctx context.Context, wire http.RoundTripper, jar http.CookieJar, proxy Proxy, method, location, body string, headers http.Header, policy func(string, int) error) (*http.Response, error) {
	headers = headers.Clone()
	for previousCount := 1; ; previousCount++ {
		request, err := redirectRequest(ctx, jar, method, location, body, headers)
		if err != nil {
			return nil, err
		}
		outbound := redirectOutbound(request, ctx, proxy)
		response, err := wire.RoundTrip(outbound)
		if err != nil {
			return nil, err
		}
		response.Request = request
		if jar != nil {
			jar.SetCookies(request.URL, response.Cookies())
		}
		status := response.StatusCode
		next, ok := nextRedirectLocation(request, response, policy != nil)
		if !ok {
			return response, nil
		}
		if err := policy(next, previousCount); err != nil {
			response.Body.Close()
			return nil, err
		}
		response.Body.Close()
		method, body = redirectMethod(status, method, body, headers)
		headers.Set("Referer", request.URL.String())
		location = next
	}
}

// redirectRequest applies jar cookies to the logical request before wire URL adaptation.
func redirectRequest(ctx context.Context, jar http.CookieJar, method, location, body string, headers http.Header) (*http.Request, error) {
	request, err := wireRequest(ctx, method, location, body, headers)
	if err != nil {
		return nil, err
	}
	if jar != nil {
		for _, cookie := range jar.Cookies(request.URL) {
			request.AddCookie(cookie)
		}
	}
	return request, nil
}

// redirectOutbound retains proxy absolute form only for HTTP destinations with HTTP proxies.
func redirectOutbound(request *http.Request, ctx context.Context, proxy Proxy) *http.Request {
	outbound := request.Clone(ctx)
	wireUrl := *request.URL
	shouldUseAbsoluteForm := false
	if address, ok := proxy.Url(); ok && wireUrl.Scheme == "http" {
		selected, _ := whatwg.NewParser().Parse(address)
		shouldUseAbsoluteForm = selected.Scheme() == "http" || selected.Scheme() == "https"
	}
	if !shouldUseAbsoluteForm {
		wireUrl.Opaque = strings.TrimPrefix(wireUrl.Opaque, "//"+wireUrl.Host)
	}
	outbound.URL = &wireUrl
	return outbound
}

// nextRedirectLocation leaves malformed or unhandled responses open without invoking policy.
func nextRedirectLocation(request *http.Request, response *http.Response, hasPolicy bool) (string, bool) {
	status := response.StatusCode
	if !hasPolicy || !isRedirectStatus(status) {
		return "", false
	}
	references, exists := response.Header["Location"]
	if !exists || len(references) == 0 {
		return "", false
	}
	next, ok := redirectLocation(request.URL.String(), references[0])
	if !ok {
		return "", false
	}
	return next, true
}

// isRedirectStatus recognizes exactly the five legacy redirect statuses.
func isRedirectStatus(status int) bool {
	return status == 301 || status == 302 || status == 303 || status == 307 || status == 308
}

// redirectMethod preserves HEAD while clearing rewritten bodies and their four content headers.
func redirectMethod(status int, method, body string, headers http.Header) (string, string) {
	if (status == 301 || status == 302) && method == http.MethodPost || status == 303 {
		if method != http.MethodHead {
			method = http.MethodGet
		}
		body = ""
		for _, name := range []string{"Content-Type", "Content-Length", "Transfer-Encoding", "Content-Encoding"} {
			headers.Del(name)
		}
	}
	return method, body
}
