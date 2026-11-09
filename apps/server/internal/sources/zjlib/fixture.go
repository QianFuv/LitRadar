package zjlib

import (
	"context"
	"fmt"
	"log/slog"
	"slices"
	"strings"
	"sync"
	"time"
)

// FixtureMode chooses deterministic upstream success or one classified failure.
type FixtureMode string

const (
	Success          FixtureMode = "success"
	StartFailure     FixtureMode = "start_failure"
	PollTimeout      FixtureMode = "poll_timeout"
	PollFailure      FixtureMode = "poll_failure"
	WarmupFailure    FixtureMode = "warmup_failure"
	FulltextMismatch FixtureMode = "fulltext_mismatch"
	FulltextFailure  FixtureMode = "fulltext_failure"
)

// ParseFixtureMode accepts only the original case-sensitive mode names and aliases.
func ParseFixtureMode(value string) (FixtureMode, bool) {
	switch strings.TrimSpace(value) {
	case "success", "live_success":
		return Success, true
	case "start_failure":
		return StartFailure, true
	case "timeout", "poll_timeout":
		return PollTimeout, true
	case "poll_failure":
		return PollFailure, true
	case "warmup_failure":
		return WarmupFailure, true
	case "fulltext_mismatch":
		return FulltextMismatch, true
	case "fulltext_failure":
		return FulltextFailure, true
	}
	return "", false
}

// FixtureTransport stores independently owned cookies and deterministic full-text responses.
type FixtureTransport struct{ state *fixtureState }
type fixtureState struct {
	mutex   sync.Mutex
	mode    FixtureMode
	cookies []Cookie
	now     func() int64
}

// NewFixtureTransport creates an empty fixture session using the selected failure mode.
func NewFixtureTransport(mode FixtureMode) *FixtureTransport {
	return &FixtureTransport{state: &fixtureState{mode: mode, cookies: []Cookie{}, now: func() int64 { return time.Now().Unix() }}}
}
func (fixture FixtureTransport) String() string {
	return "FixtureZjlibCnkiTransport { cookies: [REDACTED] }"
}
func (fixture FixtureTransport) GoString() string     { return fixture.String() }
func (fixture FixtureTransport) LogValue() slog.Value { return slog.StringValue(fixture.String()) }

// Clone copies cookies and retains the same deterministic behavior without shared mutable data.
func (fixture *FixtureTransport) Clone() *FixtureTransport {
	fixture.state.mutex.Lock()
	defer fixture.state.mutex.Unlock()
	return &FixtureTransport{state: &fixtureState{mode: fixture.state.mode, cookies: cloneCookies(fixture.state.cookies), now: fixture.state.now}}
}

// StartQrLogin returns the fixed fixture QR identity unless start failure is selected.
func (fixture *FixtureTransport) StartQrLogin(context.Context) (QrLogin, error) {
	if fixture.state.mode == StartFailure {
		return QrLogin{}, &Error{Kind: "Request", Message: "fixture QR login start failed"}
	}
	return QrLogin{Uuid: "qr-live-fixture", Status: "WAITING_SCAN", QrCode: "https://qr.test/qr-live-fixture.png"}, nil
}

// PollQrLogin preserves timeout formatting and creates the original unsigned fixture token.
func (fixture *FixtureTransport) PollQrLogin(ctx context.Context, uuid string, timeoutSeconds int64, intervalSeconds float64) (string, error) {
	switch fixture.state.mode {
	case PollTimeout:
		return "", &Error{Kind: "Timeout", Message: fmt.Sprintf("Timed out waiting for QR scan after %d seconds.", timeoutSeconds)}
	case PollFailure:
		return "", &Error{Kind: "Request", Message: "QR login ended with status FAILED."}
	}
	return unsignedJwt(fixture.state.now() + 3600), nil
}
func (fixture *FixtureTransport) upsert(cookie Cookie) {
	fixture.state.mutex.Lock()
	defer fixture.state.mutex.Unlock()
	for index, existing := range fixture.state.cookies {
		if existing.Name == cookie.Name {
			fixture.state.cookies[index] = cookie
			return
		}
	}
	fixture.state.cookies = append(fixture.state.cookies, cookie)
}

// SetLoginCookie updates the first same-name entry regardless of its prior domain.
func (fixture *FixtureTransport) SetLoginCookie(token string) {
	fixture.upsert(NewCookie("userToken", token, "www.zjlib.cn"))
}

// WarmUpFulltextSession creates the fixture's usable proxy cookie.
func (fixture *FixtureTransport) WarmUpFulltextSession(ctx context.Context, token string) (string, error) {
	if fixture.state.mode == WarmupFailure {
		return "", &Error{Kind: "Request", Message: "Share warm-up failed"}
	}
	fixture.upsert(NewCookie("vpn358_sid", "SECRET_VPN_VALUE", "http-10--18--17--173.elib.zyproxy.zjlib.cn"))
	return ProxyBase + "/kns55/", nil
}

// LoadCookies replaces the complete fixture jar with owned cookie values.
func (fixture *FixtureTransport) LoadCookies(cookies []Cookie) {
	fixture.state.mutex.Lock()
	defer fixture.state.mutex.Unlock()
	fixture.state.cookies = cloneCookies(cookies)
}

// Cookies returns a stable name-sorted snapshot preserving duplicate-name order.
func (fixture *FixtureTransport) Cookies() []Cookie {
	fixture.state.mutex.Lock()
	defer fixture.state.mutex.Unlock()
	cookies := cloneCookies(fixture.state.cookies)
	slices.SortStableFunc(cookies, func(first, second Cookie) int { return strings.Compare(first.Name, second.Name) })
	return cookies
}

// HasUnexpiredCookie accepts any same-name entry whose strict expiry boundary has not passed.
func (fixture *FixtureTransport) HasUnexpiredCookie(name string, now int64) bool {
	fixture.state.mutex.Lock()
	defer fixture.state.mutex.Unlock()
	for _, cookie := range fixture.state.cookies {
		if cookie.Name == name && cookie.IsUnexpired(now) {
			return true
		}
	}
	return false
}

// Search returns one row with the unchanged caller title whenever the limit is nonzero.
func (fixture *FixtureTransport) Search(ctx context.Context, keyword string, limit uint64) ([]SearchResult, error) {
	if limit == 0 {
		return []SearchResult{}, nil
	}
	return []SearchResult{{Index: 1, Title: keyword, DetailUrl: "https://fixture.cnki.test/detail", FileName: pointer("fixture"), DbName: pointer("CJFDLAST2026"), DbCode: pointer("CJFD"), DownloadUrl: pointer("https://fixture.cnki.test/download.aspx?dflag=pdfdown")}}, nil
}

// InspectResultMetadata preserves the fixture's exact identity or selected mismatch.
func (fixture *FixtureTransport) InspectResultMetadata(ctx context.Context, result SearchResult) (ArticleCandidate, error) {
	identity := ArticleIdentity{Title: result.Title, Authors: "Ada Lovelace; Grace Hopper", JournalTitle: "Fixture CNKI Journal"}
	if fixture.state.mode == FulltextMismatch {
		identity = ArticleIdentity{Title: "Mismatched CNKI Article", Authors: "Different Author", JournalTitle: "Different Journal"}
	}
	return ArticleCandidate{Result: cloneResult(result), Identity: identity, DetailUrl: result.DetailUrl, PdfUrl: cloneString(result.DownloadUrl)}, nil
}

// DownloadPdf returns fresh owned bytes and a title-derived filename.
func (fixture *FixtureTransport) DownloadPdf(ctx context.Context, url string, title, referer *string) (DownloadedPdf, error) {
	if fixture.state.mode == FulltextFailure {
		return DownloadedPdf{}, &Error{Kind: "Request", Message: "fixture PDF download failed"}
	}
	name := "Fixture CNKI Article"
	if title != nil {
		name = *title
	}
	content := []byte("%PDF-1.4\n% fixture cnki pdf\n")
	return DownloadedPdf{Filename: SafeFilename(name) + ".pdf", FinalUrl: url, ContentType: "application/pdf", ByteCount: uint64(len(content)), Content: content}, nil
}
