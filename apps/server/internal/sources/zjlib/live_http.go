package zjlib

import (
	"context"
	"errors"
	"fmt"
	"golang.org/x/sync/semaphore"
	"log/slog"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"

	platform "github.com/QianFuv/LitRadar/apps/server/internal/platform/transport"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

const responseMaximumBytes = 2 * 1024 * 1024
const browserUserAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36"

// LiveConfig bounds each HTTP request and each downloaded document.
type LiveConfig struct {
	TimeoutSeconds       uint64
	MaximumDocumentBytes int64
}

// DefaultLiveConfig supplies the original thirty-second and thirty-two-MiB limits.
func DefaultLiveConfig() LiveConfig {
	return LiveConfig{TimeoutSeconds: 30, MaximumDocumentBytes: DefaultMaximumDocumentBytes}
}

// LiveTransport owns one upstream session, explicit networking and a shared article deadline.
type LiveTransport struct{ state *liveState }
type liveState struct {
	gate         *semaphore.Weighted
	redirect     *http.Client
	allowed      endpoints
	config       LiveConfig
	deadline     time.Time
	proxy        transport.Proxy
	lastBriefURL *string
}

// NewLiveTransport creates the production endpoint policy and an independent cookie jar.
func NewLiveTransport(config LiveConfig, proxy transport.Proxy, deadline time.Time) (*LiveTransport, error) {
	return newLiveTransport(config, proxy, deadline, defaultEndpoints())
}
func newLiveTransport(config LiveConfig, proxy transport.Proxy, deadline time.Time, allowed endpoints) (*LiveTransport, error) {
	wire, err := proxy.ClientTransport()
	if err != nil {
		return nil, &Error{Kind: "Request", Message: err.Error()}
	}
	config.TimeoutSeconds = max(config.TimeoutSeconds, 1)
	config.MaximumDocumentBytes = max(config.MaximumDocumentBytes, 1)
	jar := platform.NewCookieJar()
	redirect := &http.Client{Transport: wire, Jar: jar}
	return &LiveTransport{state: &liveState{gate: semaphore.NewWeighted(1), redirect: redirect, allowed: allowed, config: config, deadline: deadline, proxy: proxy}}, nil
}
func (live LiveTransport) String() string       { return "LiveZjlibCnkiTransport { session: [REDACTED] }" }
func (live LiveTransport) GoString() string     { return live.String() }
func (live LiveTransport) LogValue() slog.Value { return slog.StringValue(live.String()) }
func (live *LiveTransport) enter(ctx context.Context) error {
	if err := live.state.gate.Acquire(ctx, 1); err != nil {
		return &Error{Kind: "Request", Message: transport.ErrArticleDeadline.Error()}
	}
	return nil
}
func (live *LiveTransport) leave() { live.state.gate.Release(1) }

// Clone shares the original HTTP cookie clients and copies the last search referer.
func (live *LiveTransport) Clone() *LiveTransport {
	_ = live.state.gate.Acquire(context.Background(), 1)
	defer live.leave()
	return &LiveTransport{state: &liveState{gate: semaphore.NewWeighted(1), redirect: live.state.redirect, allowed: live.state.allowed, config: live.state.config, deadline: live.state.deadline, proxy: live.state.proxy, lastBriefURL: cloneString(live.state.lastBriefURL)}}
}

// Close releases idle pooled connections without discarding session state.
func (live *LiveTransport) Close() { live.state.redirect.CloseIdleConnections() }
func (live *LiveTransport) send(ctx context.Context, method, location, body string, headers http.Header, redirect bool) (*http.Response, context.CancelFunc, error) {
	timeout, err := transport.RequestTimeout(transport.Delay{Seconds: live.state.config.TimeoutSeconds}.Duration(), live.state.deadline)
	if err != nil || ctx.Err() != nil {
		return nil, func() {}, &Error{Kind: "Request", Message: transport.ErrArticleDeadline.Error()}
	}
	requestCtx, cancel := context.WithTimeout(ctx, timeout)
	response, err := live.roundTripSequence(requestCtx, method, location, body, headers, redirect)
	if err != nil {
		cancel()
		return nil, func() {}, err
	}
	finish := func() { _ = response.Body.Close(); cancel() }
	return response, finish, nil
}
func redactURL(value string) string {
	parsed, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return "<invalid-url>"
	}
	parsed.SetSearch("")
	parsed.SetHash("")
	return parsed.Href(false)
}
func raiseForStatus(response *http.Response, action string) error {
	if response.StatusCode >= 400 {
		return &Error{Kind: "Request", Message: fmt.Sprintf("%s failed with HTTP %d: %s", action, response.StatusCode, redactURL(response.Request.URL.String()))}
	}
	return nil
}
func responseText(response *http.Response) (string, error) {
	value, err := transport.BoundedText(response, responseMaximumBytes)
	if err == nil {
		return value, nil
	}
	if errors.Is(err, transport.ErrTooLarge) {
		return "", &Error{Kind: "Request", Message: "ZJLib response exceeded the configured size limit."}
	}
	return "", &Error{Kind: "Request", Message: err.Error()}
}
func jsonPayload(response *http.Response, action string) (any, error) {
	if err := raiseForStatus(response, action); err != nil {
		return nil, err
	}
	value, err := transport.BoundedJSON(response, responseMaximumBytes)
	if err != nil {
		switch {
		case errors.Is(err, transport.ErrTooLarge):
			return nil, &Error{Kind: "Parse", Message: action + " response exceeded the configured size limit."}
		case errors.Is(err, transport.ErrReadFailed):
			return nil, &Error{Kind: "Request", Message: action + " response body could not be read."}
		default:
			return nil, &Error{Kind: "Parse", Message: action + " returned non-JSON response."}
		}
	}
	if success, ok := field(value, "success").(bool); ok && !success {
		return nil, &Error{Kind: "Request", Message: action + " failed."}
	}
	if _, ok := value.(map[string]any); !ok {
		return nil, &Error{Kind: "Parse", Message: action + " returned non-object JSON response."}
	}
	return value, nil
}
func payloadData(value any, action string) (any, error) {
	data, ok := field(value, "data").(map[string]any)
	if !ok {
		return nil, &Error{Kind: "Parse", Message: action + " response did not contain object data."}
	}
	return data, nil
}
func validHeader(value string) bool {
	for _, character := range []byte(value) {
		if character < 32 && character != '\t' || character == 127 {
			return false
		}
	}
	return true
}
func baseHeaders() http.Header {
	return http.Header{"User-Agent": {browserUserAgent}, "Accept-Language": {"zh-CN;q=0.9"}}
}
func htmlHeaders(referer string) http.Header {
	headers := baseHeaders()
	headers.Set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
	if validHeader(referer) {
		headers.Set("Referer", referer)
	}
	return headers
}
func ajaxHeaders(referer string) http.Header {
	headers := htmlHeaders(referer)
	headers.Set("Accept", "*/*")
	headers.Set("X-Requested-With", "XMLHttpRequest")
	return headers
}
func wwwHeaders(base string, token *string) http.Header {
	headers := baseHeaders()
	headers.Set("Accept", "*/*")
	headers.Set("Referer", strings.TrimRight(base, "/"))
	headers.Set("bff-org-id", "1916318653650423810")
	if token != nil && validHeader(*token) {
		headers.Set("bff-user-token", *token)
	}
	return headers
}
func formHeaders(referer, origin string) http.Header {
	headers := htmlHeaders(referer)
	headers.Set("Content-Type", "application/x-www-form-urlencoded")
	if validHeader(origin) {
		headers.Set("Origin", origin)
	}
	return headers
}

// SetLoginCookie installs the browser token under the configured library origin.
func (live *LiveTransport) SetLoginCookie(token string) {
	location, _ := url.Parse(live.state.allowed.bases[wwwFamily])
	raw := "userToken=" + token + "; Path=/"
	if location.Scheme == "https" {
		raw += "; Secure"
	}
	if cookie, err := http.ParseSetCookie(raw); err == nil {
		live.state.redirect.Jar.SetCookies(location, []*http.Cookie{cookie})
	}
}

// LoadCookies merges persisted cookies using the original domain/path policy and omission of expiry metadata.
func (live *LiveTransport) LoadCookies(cookies []Cookie) {
	for _, cookie := range cookies {
		if strings.TrimSpace(cookie.Name) == "" {
			continue
		}
		location := live.cookieRestoreOrigin(cookie)
		parsed, err := url.Parse(location)
		if err != nil {
			continue
		}
		raw := restoredCookieHeader(cookie)
		if parsedCookie, err := http.ParseSetCookie(raw); err == nil {
			live.state.redirect.Jar.SetCookies(parsed, []*http.Cookie{parsedCookie})
		}
	}
}

// Cookies snapshots names visible at known origins, retaining the original last-domain-wins projection.
func (live *LiveTransport) Cookies() []Cookie {
	values := map[string]Cookie{}
	for _, base := range live.state.allowed.bases {
		location, _ := url.Parse(base)
		for _, cookie := range live.state.redirect.Jar.Cookies(location) {
			name := strings.TrimSpace(cookie.Name)
			if name != "" {
				values[name] = NewCookie(name, strings.TrimSpace(cookie.Value), location.Hostname())
			}
		}
	}
	names := make([]string, 0, len(values))
	for name := range values {
		names = append(names, name)
	}
	sort.Strings(names)
	result := make([]Cookie, 0, len(names))
	for _, name := range names {
		result = append(result, values[name])
	}
	return result
}

// HasUnexpiredCookie observes the jar's own expiry clock, as the original live transport does.
func (live *LiveTransport) HasUnexpiredCookie(name string, _ int64) bool {
	for _, cookie := range live.Cookies() {
		if cookie.Name == name {
			return true
		}
	}
	return false
}

// cookieRestoreOrigin chooses a configured base by host while retaining the empty-domain default.
func (live *LiveTransport) cookieRestoreOrigin(cookie Cookie) string {
	host := strings.TrimLeft(strings.TrimSpace(cookie.Domain), ".")
	location := "https://" + host + "/"
	if host == "" {
		location = live.state.allowed.bases[wwwFamily]
	} else {
		for _, base := range live.state.allowed.bases {
			parsed, _ := url.Parse(base)
			if parsed.Hostname() == host {
				location = base
				break
			}
		}
	}
	return location
}

// restoredCookieHeader rebuilds only name, value, path, domain and secure metadata.
func restoredCookieHeader(cookie Cookie) string {
	raw := cookie.Name + "=" + cookie.Value + "; Path=" + cookie.Path
	if strings.TrimSpace(cookie.Domain) != "" {
		raw += "; Domain=" + cookie.Domain
	}
	if cookie.Secure {
		raw += "; Secure"
	}
	return raw
}
