package zjlib

import (
	"context"
	"fmt"
	"log/slog"
	"math"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func appendEndpoint(base, path string) string         { return strings.TrimRight(base, "/") + path }
func encodePairs(pairs ...scholarly.QueryPair) string { return scholarly.EncodeQuery(pairs) }
func pair(name, value string) scholarly.QueryPair {
	return scholarly.QueryPair{Name: name, Value: value}
}
func origin(value string) string {
	parsed, _ := whatwg.NewParser().Parse(value)
	return parsed.Scheme() + "://" + parsed.Host()
}
func (live *LiveTransport) sleep(ctx context.Context, delay time.Duration) error {
	if err := transport.ArticleSleep(ctx, delay, live.state.deadline); err != nil {
		return &Error{Kind: "Request", Message: transport.ErrArticleDeadline.Error()}
	}
	return nil
}

// StartQrLogin retrieves and validates the initial QR challenge without following redirects.
func (live *LiveTransport) StartQrLogin(ctx context.Context) (QrLogin, error) {
	if err := live.enter(ctx); err != nil {
		return QrLogin{}, err
	}
	defer live.leave()
	location := appendEndpoint(live.state.allowed.bases[wwwFamily], "/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr")
	response, finish, err := live.send(ctx, http.MethodGet, location, "", wwwHeaders(live.state.allowed.bases[wwwFamily], nil), false)
	if err != nil {
		return QrLogin{}, err
	}
	defer finish()
	payload, err := jsonPayload(response, "start QR login")
	if err != nil {
		return QrLogin{}, err
	}
	data, err := payloadData(payload, "start QR login")
	if err != nil {
		return QrLogin{}, err
	}
	login := QrLogin{Uuid: stringField(data, "uuid"), Status: stringField(data, "status"), QrCode: stringField(data, "qrCode")}
	if login.Uuid == "" || login.QrCode == "" {
		return QrLogin{}, &Error{Kind: "Parse", Message: "QR login response did not contain uuid/qrCode."}
	}
	return login, nil
}

// PollQrLogin preserves terminal statuses and the independent QR polling timeout.
func (live *LiveTransport) PollQrLogin(ctx context.Context, uuid string, timeoutSeconds int64, intervalSeconds float64) (string, error) {
	if err := live.enter(ctx); err != nil {
		return "", err
	}
	defer live.leave()
	timeout := transport.Delay{Seconds: uint64(max(timeoutSeconds, 1))}.Duration()
	deadline := time.Now().Add(timeout)
	if math.IsNaN(intervalSeconds) {
		intervalSeconds = 0.1
	}
	interval := time.Duration(min(max(intervalSeconds, 0.1), 10) * float64(time.Second))
	location := appendEndpoint(live.state.allowed.bases[wwwFamily], "/bff-api/reader-sso-service/portal-pc-api/qr/status") + "?" + encodePairs(pair("uuid", uuid))
	for time.Now().Before(deadline) {
		response, finish, err := live.send(ctx, http.MethodGet, location, "", wwwHeaders(live.state.allowed.bases[wwwFamily], nil), false)
		if err != nil {
			return "", err
		}
		payload, err := jsonPayload(response, "poll QR login")
		finish()
		if err != nil {
			return "", err
		}
		data, err := payloadData(payload, "poll QR login")
		if err != nil {
			return "", err
		}
		status := stringField(data, "status")
		if status == "COMPLETE" {
			token := stringField(data, "data")
			if token == "" {
				return "", &Error{Kind: "Parse", Message: "QR login completed but did not return token."}
			}
			return token, nil
		}
		if slices.Contains([]string{"EXPIRED", "CANCEL", "CANCELED", "FAIL", "FAILED"}, status) {
			return "", &Error{Kind: "Request", Message: "QR login ended with status " + status + "."}
		}
		remaining := time.Until(deadline)
		if remaining <= 0 {
			break
		}
		if err := live.sleep(ctx, min(interval, remaining)); err != nil {
			return "", err
		}
	}
	return "", &Error{Kind: "Timeout", Message: fmt.Sprintf("Timed out waiting for QR scan after %d seconds.", timeoutSeconds)}
}
func (live *LiveTransport) buildShareSsoUrl(ctx context.Context, token string) (string, error) {
	location := appendEndpoint(live.state.allowed.bases[wwwFamily], "/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl") + "?" + encodePairs(pair("referURL", live.state.allowed.entry))
	response, finish, err := live.send(ctx, http.MethodGet, location, "", wwwHeaders(live.state.allowed.bases[wwwFamily], &token), false)
	if err != nil {
		return "", err
	}
	defer finish()
	payload, err := jsonPayload(response, "build Share SSO URL")
	if err != nil {
		return "", err
	}
	parsed, err := live.state.allowed.parse(stringField(payload, "data"), shareFamily)
	if err != nil {
		return "", err
	}
	return parsed.Href(false), nil
}
func (live *LiveTransport) enterShare(ctx context.Context, ssoUrl string) error {
	response, finish, err := live.send(ctx, http.MethodGet, ssoUrl, "", htmlHeaders(live.state.allowed.bases[wwwFamily]+"/"), false)
	if err != nil {
		return err
	}
	if err = raiseForStatus(response, "enter Share protocolAuth"); err != nil {
		finish()
		return err
	}
	responseUrl := response.Request.URL.String()
	text, err := responseText(response)
	finish()
	if err != nil {
		return err
	}
	sync, err := extractShareCookieSync(text, live.state.allowed)
	if err != nil {
		return err
	}
	if sync != nil {
		headers := htmlHeaders(responseUrl)
		headers.Set("Origin", origin(live.state.allowed.bases[shareFamily]))
		headers.Set("Content-Type", "application/x-www-form-urlencoded")
		response, finish, err = live.send(ctx, http.MethodPost, sync.url, encodePairs(pair("sign", sync.fields["sign"]), pair("url", sync.fields["url"])), headers, true)
		if err != nil {
			return err
		}
		err = raiseForStatus(response, "sync Share login cookies")
		finish()
		if err != nil {
			return err
		}
	}
	response, finish, err = live.send(ctx, http.MethodGet, live.state.allowed.entry, "", htmlHeaders(ssoUrl), true)
	if err != nil {
		return err
	}
	err = raiseForStatus(response, "open Share entry")
	finish()
	if err != nil {
		return err
	}
	location := appendEndpoint(live.state.allowed.bases[shareFamily], "/engine2/header/user-info") + "?" + encodePairs(pair("t", strconv.FormatInt(time.Now().UnixMilli(), 10)))
	response, finish, err = live.send(ctx, http.MethodGet, location, "", ajaxHeaders(live.state.allowed.entry), true)
	if err != nil {
		return err
	}
	defer finish()
	return raiseForStatus(response, "load Share user info")
}
func (live *LiveTransport) getProxyLoginUrl(ctx context.Context) (string, error) {
	location := appendEndpoint(live.state.allowed.bases[shareFamily], "/sso/api/auth/library/vpn358") + "?" + encodePairs(pair("wfwfid", "2120"), pair("refer", live.state.allowed.referer))
	response, finish, err := live.send(ctx, http.MethodGet, location, "", htmlHeaders(live.state.allowed.entry), false)
	if err != nil {
		return "", err
	}
	defer finish()
	if err = raiseForStatus(response, "get zyproxy login URL"); err != nil {
		return "", err
	}
	var login string
	if location, ok := response.Header["Location"]; ok && len(location) > 0 && asciiHeader(location[0]) {
		login, err = joinUrl(response.Request.URL.String(), location[0])
	} else {
		var text string
		text, err = responseText(response)
		if err == nil {
			login, err = extractWindowLocation(text, response.Request.URL.String())
		}
	}
	if err != nil {
		return "", err
	}
	parsed, err := live.state.allowed.parse(login, loginFamily)
	if err != nil {
		return "", err
	}
	return parsed.Href(false), nil
}
func asciiHeader(value string) bool {
	for _, character := range []byte(value) {
		if character >= 128 {
			return false
		}
	}
	return true
}

type proxyEndpoint struct {
	family endpointFamily
	path   string
}

func (allowed endpoints) proxyEndpoint(location *whatwg.Url) (proxyEndpoint, error) {
	if location.Username() != "" || location.Password() != "" {
		return proxyEndpoint{}, &Error{Kind: "Parse", Message: "zyproxy redirect used an unexpected origin."}
	}
	family, err := allowed.family(location)
	if err != nil || family != loginFamily && family != proxyFamily {
		return proxyEndpoint{}, &Error{Kind: "Parse", Message: "zyproxy redirect used an unexpected endpoint."}
	}
	base, _ := whatwg.NewParser().Parse(allowed.bases[family])
	path := strings.TrimRight(strings.TrimPrefix(location.Pathname(), strings.TrimRight(base.Pathname(), "/")), "/")
	if path == "" {
		path = "/"
	}
	return proxyEndpoint{family: family, path: asciiLower(path)}, nil
}
func expectedLoop(first, second proxyEndpoint) bool {
	return first == (proxyEndpoint{family: loginFamily, path: "/index.php"}) && second == (proxyEndpoint{family: proxyFamily, path: "/kns55"}) || second == (proxyEndpoint{family: loginFamily, path: "/index.php"}) && first == (proxyEndpoint{family: proxyFamily, path: "/kns55"})
}
func (live *LiveTransport) enterProxy(ctx context.Context, login string) (string, bool, error) {
	current, err := whatwg.NewParser().Parse(login)
	if err != nil {
		return "", false, &Error{Kind: "Parse", Message: "zyproxy login URL was invalid."}
	}
	if _, err = live.state.allowed.proxyEndpoint(current); err != nil {
		return "", false, err
	}
	history := []proxyEndpoint{}
	referer := live.state.allowed.bases[shareFamily] + "/"
	for hops := 0; ; hops++ {
		response, finish, err := live.send(ctx, http.MethodGet, current.Href(false), "", htmlHeaders(referer), false)
		if err != nil {
			return "", false, err
		}
		status := response.StatusCode
		values, hasLocation := response.Header["Location"]
		responseUrl := response.Request.URL.String()
		finish()
		if status >= 200 && status < 300 {
			endpoint, err := live.state.allowed.proxyEndpoint(current)
			if err != nil {
				return "", false, err
			}
			if endpoint.family != proxyFamily {
				return "", false, &Error{Kind: "Parse", Message: "zyproxy login ended outside the proxy host."}
			}
			if !live.HasUnexpiredCookie("vpn358_sid", time.Now().Unix()) {
				return "", false, &Error{Kind: "Parse", Message: "zyproxy login did not set vpn358_sid."}
			}
			return responseUrl, false, nil
		}
		if status < 300 || status >= 400 {
			return "", false, &Error{Kind: "Request", Message: fmt.Sprintf("enter zyproxy failed with HTTP %d.", status)}
		}
		if hops >= 4 {
			return "", false, &Error{Kind: "Request", Message: "zyproxy login exceeded 4 redirect hops."}
		}
		if !hasLocation || len(values) == 0 || !asciiHeader(values[0]) {
			return "", false, &Error{Kind: "Parse", Message: "zyproxy redirect did not contain a valid Location."}
		}
		next, err := whatwg.NewParser().ParseRef(responseUrl, values[0])
		if err != nil {
			return "", false, &Error{Kind: "Parse", Message: "zyproxy redirect Location was invalid."}
		}
		currentEndpoint, err := live.state.allowed.proxyEndpoint(current)
		if err != nil {
			return "", false, err
		}
		nextEndpoint, err := live.state.allowed.proxyEndpoint(next)
		if err != nil {
			return "", false, err
		}
		if currentEndpoint == nextEndpoint {
			return "", false, &Error{Kind: "Request", Message: "zyproxy returned an unexpected self-redirect."}
		}
		if slices.Contains(history, nextEndpoint) {
			if history[len(history)-1] == nextEndpoint && expectedLoop(currentEndpoint, nextEndpoint) {
				return "", true, nil
			}
			return "", false, &Error{Kind: "Request", Message: "zyproxy returned an unexpected redirect cycle."}
		}
		history = append(history, currentEndpoint)
		referer = responseUrl
		current = next
	}
}

// WarmUpFulltextSession establishes Share cookies and retries only the recognized proxy login loop.
func (live *LiveTransport) WarmUpFulltextSession(ctx context.Context, token string) (string, error) {
	if err := live.enter(ctx); err != nil {
		return "", err
	}
	defer live.leave()
	sso, err := live.buildShareSsoUrl(ctx, token)
	if err != nil {
		return "", err
	}
	if err = live.enterShare(ctx, sso); err != nil {
		return "", err
	}
	for attempt := 1; attempt <= 3; attempt++ {
		started := time.Now()
		login, err := live.getProxyLoginUrl(ctx)
		var final string
		var shouldRetry bool
		if err == nil {
			final, shouldRetry, err = live.enterProxy(ctx, login)
		}
		if err == nil && !shouldRetry {
			return final, nil
		}
		kind := "session_not_ready"
		if err != nil {
			kind = strings.ToLower(err.(*Error).Kind)
		}
		attributes := []any{"event", "source.request.failed", "component", "source", "provider", "zjlib", "endpoint", "zyproxy_login", "attempt", attempt, "outcome", "failure", "error_kind", kind, "http_status", 0, "has_http_status", false, "is_retry", attempt > 1, "will_retry", err == nil && attempt < 3, "duration_ms", time.Since(started).Milliseconds()}
		if err == nil && attempt < 3 {
			attributes = append(attributes, "retry_delay_ms", 200*attempt)
		}
		slog.WarnContext(ctx, "source.request.failed", attributes...)
		if err != nil {
			return "", err
		}
		if attempt == 3 {
			return "", &Error{Kind: "Request", Message: "zyproxy session was not accepted after 3 login attempts."}
		}
		if err = live.sleep(ctx, time.Duration(200*attempt)*time.Millisecond); err != nil {
			return "", err
		}
	}
	return "", &Error{Kind: "Request", Message: "zyproxy login attempt budget was exhausted."}
}
