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
		request, err := wireRequest(ctx, method, location, body, headers)
		if err != nil {
			return nil, err
		}
		if jar != nil {
			for _, cookie := range jar.Cookies(request.URL) {
				request.AddCookie(cookie)
			}
		}
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
		response, err := wire.RoundTrip(outbound)
		if err != nil {
			return nil, err
		}
		response.Request = request
		if jar != nil {
			jar.SetCookies(request.URL, response.Cookies())
		}
		status := response.StatusCode
		if policy == nil || status != 301 && status != 302 && status != 303 && status != 307 && status != 308 {
			return response, nil
		}
		references, exists := response.Header["Location"]
		if !exists || len(references) == 0 {
			return response, nil
		}
		next, ok := redirectLocation(request.URL.String(), references[0])
		if !ok {
			return response, nil
		}
		if err := policy(next, previousCount); err != nil {
			response.Body.Close()
			return nil, err
		}
		response.Body.Close()
		if (status == 301 || status == 302) && method == http.MethodPost || status == 303 {
			if method != http.MethodHead {
				method = http.MethodGet
			}
			body = ""
			for _, name := range []string{"Content-Type", "Content-Length", "Transfer-Encoding", "Content-Encoding"} {
				headers.Del(name)
			}
		}
		headers.Set("Referer", request.URL.String())
		location = next
	}
}
