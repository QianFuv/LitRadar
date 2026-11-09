package zjlib

import (
	"context"
	"fmt"
	"log/slog"
	"strings"
	"time"
)

// Client serializes session transitions and keeps credentials out of implicit formatting.
type Client struct{ state *clientState }
type clientState struct {
	gate                         chan struct{}
	transport                    Transport
	token, qrUuid, finalProxyUrl *string
	warmedAt                     *int64
	now                          func() int64
}

// NewClient binds a transport to empty, private login state.
func NewClient(transport Transport) *Client {
	return &Client{state: &clientState{gate: make(chan struct{}, 1), transport: transport, now: func() int64 { return time.Now().Unix() }}}
}
func (client Client) String() string       { return "ZhejiangLibraryCnkiClient { session: [REDACTED] }" }
func (client Client) GoString() string     { return client.String() }
func (client Client) LogValue() slog.Value { return slog.StringValue(client.String()) }
func (client *Client) enter(ctx context.Context) error {
	if ctx.Err() != nil {
		return &Error{Kind: "Request", Message: "article access deadline expired"}
	}
	select {
	case client.state.gate <- struct{}{}:
		if ctx.Err() != nil {
			client.leave()
			return &Error{Kind: "Request", Message: "article access deadline expired"}
		}
		return nil
	case <-ctx.Done():
		return &Error{Kind: "Request", Message: "article access deadline expired"}
	}
}
func (client *Client) leave() { <-client.state.gate }

// LoadStateData replaces token, QR, warm-up state and cookies using the original permissive projection.
func (client *Client) LoadStateData(value any) {
	client.enter(context.Background())
	defer client.leave()
	client.state.token = textField(value, "bff_user_token")
	client.state.qrUuid = textField(value, "qr_uuid")
	client.state.warmedAt = int64Field(field(value, "fulltext_warmed_at"))
	client.state.finalProxyUrl = textField(value, "final_zyproxy_url")
	cookies := []Cookie{}
	if entries, ok := field(value, "cookies").([]any); ok {
		for _, entry := range entries {
			if cookie := CookieFromJson(entry); cookie != nil {
				cookies = append(cookies, *cookie)
			}
		}
	}
	client.state.transport.LoadCookies(cookies)
}

// StateData explicitly exports private session state for encrypted persistence.
func (client *Client) StateData() map[string]any {
	client.enter(context.Background())
	defer client.leave()
	cookies := []any{}
	for _, cookie := range client.state.transport.Cookies() {
		cookies = append(cookies, cookie.Json())
	}
	var warmed any
	if client.state.warmedAt != nil {
		warmed = *client.state.warmedAt
	}
	return map[string]any{"bff_user_token": optionalValue(client.state.token), "qr_uuid": optionalValue(client.state.qrUuid), "cookies": cookies, "fulltext_warmed_at": warmed, "final_zyproxy_url": optionalValue(client.state.finalProxyUrl), "saved_at": client.state.now()}
}

// StartQrLogin changes only the retained QR id after a successful upstream response.
func (client *Client) StartQrLogin(ctx context.Context) (QrLogin, error) {
	if err := client.enter(ctx); err != nil {
		return QrLogin{}, err
	}
	defer client.leave()
	login, err := observe(ctx, "qr_login_start", 1, func() (QrLogin, error) { return client.state.transport.StartQrLogin(ctx) })
	if err == nil {
		client.state.qrUuid = pointer(login.Uuid)
	}
	return login, err
}

// PollQrLogin requires an existing QR id and retains the completed token and browser cookie.
func (client *Client) PollQrLogin(ctx context.Context, timeout int64, interval float64) (string, error) {
	if err := client.enter(ctx); err != nil {
		return "", err
	}
	defer client.leave()
	return client.poll(ctx, timeout, interval)
}
func (client *Client) poll(ctx context.Context, timeout int64, interval float64) (string, error) {
	if client.state.qrUuid == nil || strings.TrimSpace(*client.state.qrUuid) == "" {
		return "", &Error{Kind: "Request", Message: "No QR uuid available. Run start-login first."}
	}
	token, err := observe(ctx, "qr_login_poll", 1, func() (string, error) {
		return client.state.transport.PollQrLogin(ctx, *client.state.qrUuid, timeout, interval)
	})
	if err != nil {
		return "", err
	}
	client.state.token = pointer(token)
	client.state.transport.SetLoginCookie(token)
	return token, nil
}
func expiresSoon(expires, now int64) bool {
	return expires <= now || uint64(expires)-uint64(now) <= 300
}
func (client *Client) isFresh(now int64) bool {
	if client.state.warmedAt == nil || now < *client.state.warmedAt || uint64(now)-uint64(*client.state.warmedAt) >= 3600 {
		return false
	}
	if client.state.token != nil {
		if expiration := jwtExpiration(*client.state.token); expiration != nil && expiresSoon(*expiration, now) {
			return false
		}
	}
	return client.state.transport.HasUnexpiredCookie("vpn358_sid", now)
}

// HasFreshFulltextSession checks one-hour warmth, known JWT expiry and the proxy session cookie.
func (client *Client) HasFreshFulltextSession(now *int64) bool {
	client.enter(context.Background())
	defer client.leave()
	instant := client.state.now()
	if now != nil {
		instant = *now
	}
	return client.isFresh(instant)
}

// WarmUpFulltextSession reuses fresh state or completes login before warming proxy cookies.
func (client *Client) WarmUpFulltextSession(ctx context.Context) (string, error) {
	if err := client.enter(ctx); err != nil {
		return "", err
	}
	defer client.leave()
	if client.isFresh(client.state.now()) {
		slog.DebugContext(ctx, "source.session.reused", "event", "source.session.reused", "component", "source", "provider", "zjlib", "endpoint", "fulltext_session")
		if client.state.finalProxyUrl != nil {
			return *client.state.finalProxyUrl, nil
		}
		return ProxyBase + "/kns55/", nil
	}
	var token string
	if client.state.token != nil {
		token = *client.state.token
		if expires := jwtExpiration(token); expires != nil && expiresSoon(*expires, client.state.now()) {
			return "", &Error{Kind: "Request", Message: "bff-user-token is expired or expires soon. Run QR login again."}
		}
	} else {
		var err error
		token, err = client.poll(ctx, 180, 2)
		if err != nil {
			return "", err
		}
	}
	client.state.transport.SetLoginCookie(token)
	final, err := observe(ctx, "fulltext_session", 1, func() (string, error) { return client.state.transport.WarmUpFulltextSession(ctx, token) })
	if err != nil {
		return "", err
	}
	now := client.state.now()
	client.state.warmedAt = &now
	client.state.finalProxyUrl = pointer(final)
	return final, nil
}

// DownloadMatchingPdf advances only past mismatches or missing PDF links; transport failures stop fallback.
func (client *Client) DownloadMatchingPdf(ctx context.Context, expected ArticleIdentity, limit uint64) (DownloadedPdf, error) {
	if err := client.enter(ctx); err != nil {
		return DownloadedPdf{}, err
	}
	defer client.leave()
	started := time.Now()
	results, err := observe(ctx, "fulltext_search", 1, func() ([]SearchResult, error) { return client.state.transport.Search(ctx, expected.Title, limit) })
	if err != nil {
		return DownloadedPdf{}, err
	}
	failures := []string{}
	for index, result := range results {
		attempt := uint64(index + 1)
		candidate, err := observe(ctx, "fulltext_metadata", attempt, func() (ArticleCandidate, error) { return client.state.transport.InspectResultMetadata(ctx, result) })
		if err != nil {
			return DownloadedPdf{}, err
		}
		reason, message := "", ""
		if !MatchesArticle(expected, candidate.Identity) {
			reason, message = "metadata_mismatch", "metadata mismatch"
		} else if candidate.PdfUrl == nil {
			reason, message = "pdf_link_missing", "PDF link missing"
		}
		if reason != "" {
			slog.DebugContext(ctx, "source.fallback.activated", "event", "source.fallback.activated", "component", "source", "provider", "zjlib", "endpoint", "fulltext_match", "reason", reason, "fallback", "next_candidate", "attempt", attempt)
			failures = append(failures, fmt.Sprintf("%d: %s", result.Index, message))
			continue
		}
		download, err := observe(ctx, "fulltext_download", attempt, func() (DownloadedPdf, error) {
			return client.state.transport.DownloadPdf(ctx, *candidate.PdfUrl, &candidate.Identity.Title, &candidate.DetailUrl)
		})
		if err != nil {
			return DownloadedPdf{}, err
		}
		slog.InfoContext(ctx, "source.fulltext.completed", "event", "source.fulltext.completed", "component", "source", "provider", "zjlib", "outcome", "success", "inspected_candidate_count", attempt, "byte_count", len(download.Content), "duration_ms", time.Since(started).Milliseconds())
		return download, nil
	}
	slog.WarnContext(ctx, "source.fulltext.failed", "event", "source.fulltext.failed", "component", "source", "provider", "zjlib", "outcome", "failure", "error_kind", "no_exact_match", "inspected_candidate_count", len(failures), "duration_ms", time.Since(started).Milliseconds())
	detail := "no search results"
	if len(failures) > 0 {
		detail = strings.Join(failures, " | ")
	}
	return DownloadedPdf{}, &Error{Kind: "Request", Message: "No exact CNKI full-text match found: " + detail}
}
func observe[T any](ctx context.Context, endpoint string, attempt uint64, operation func() (T, error)) (T, error) {
	started := time.Now()
	result, err := operation()
	attributes := []any{"component", "source", "provider", "zjlib", "endpoint", endpoint, "attempt", attempt, "http_status", 0, "has_http_status", false, "is_retry", attempt > 1, "will_retry", false, "duration_ms", time.Since(started).Milliseconds()}
	if err == nil {
		attributes = append(attributes, "event", "source.request.completed", "outcome", "success")
		slog.DebugContext(ctx, "source.request.completed", attributes...)
	} else {
		kind := "request"
		if failure, ok := err.(*Error); ok {
			kind = strings.ToLower(failure.Kind)
		}
		attributes = append(attributes, "event", "source.request.failed", "outcome", "failure", "error_kind", kind)
		slog.WarnContext(ctx, "source.request.failed", attributes...)
	}
	return result, err
}
