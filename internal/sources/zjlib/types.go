// Package zjlib provides Zhejiang Library mediated CNKI login and full-text access.
package zjlib

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

const WwwBase = "https://www.zjlib.cn"
const ShareBase = "https://share.zjlib.cn"
const ProxyBase = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn"
const LoginBase = "https://login.elib.zyproxy.zjlib.cn"
const DefaultMaximumDocumentBytes = 32 * 1024 * 1024

// Error retains upstream request, parsing and QR timeout classifications.
type Error struct{ Kind, Message string }

func (failure *Error) Error() string { return failure.Message }

// IsTimeout identifies the QR polling timeout outcome.
func (failure *Error) IsTimeout() bool { return failure.Kind == "Timeout" }

// QrLogin describes the newly created QR challenge.
type QrLogin struct {
	Uuid   string `json:"uuid"`
	Status string `json:"status"`
	QrCode string `json:"qr_code"`
}

// ArticleIdentity contains the three independent fields required for an exact full-text match.
type ArticleIdentity struct {
	Title        string `json:"title"`
	Authors      string `json:"authors"`
	JournalTitle string `json:"journal_title"`
}

// SearchResult preserves upstream page order and optional row-level identifiers.
type SearchResult struct {
	Index       uint64  `json:"index"`
	Title       string  `json:"title"`
	DetailUrl   string  `json:"detail_url"`
	FileName    *string `json:"file_name"`
	DbName      *string `json:"db_name"`
	DbCode      *string `json:"db_code"`
	DownloadUrl *string `json:"download_url"`
}

// ArticleCandidate associates inspected metadata and a PDF link with its original result.
type ArticleCandidate struct {
	Result    SearchResult    `json:"result"`
	Identity  ArticleIdentity `json:"identity"`
	DetailUrl string          `json:"detail_url"`
	PdfUrl    *string         `json:"pdf_url"`
}

// DownloadedPdf owns a bounded document and its response metadata.
type DownloadedPdf struct {
	Filename    string `json:"filename"`
	FinalUrl    string `json:"final_url"`
	ContentType string `json:"content_type"`
	ByteCount   uint64 `json:"byte_count"`
	Content     []byte `json:"content"`
}

// Cookie is private persisted session data; implicit formatting redacts its value.
type Cookie struct {
	Name, Value, Domain, Path string
	Secure                    bool
	Expires                   *int64
	Discard                   bool
}

func (cookie Cookie) String() string {
	return fmt.Sprintf("ZjlibCnkiCookie { name: %q, value: [REDACTED], domain: %q, path: %q, secure: %t }", cookie.Name, cookie.Domain, cookie.Path, cookie.Secure)
}
func (cookie Cookie) GoString() string     { return cookie.String() }
func (cookie Cookie) LogValue() slog.Value { return slog.StringValue(cookie.String()) }

// NewCookie creates a secure persistent cookie with the original default path.
func NewCookie(name, value, domain string) Cookie {
	return Cookie{Name: name, Value: value, Domain: domain, Path: "/", Secure: true}
}

// CookieFromJson ignores malformed entries and preserves original field defaulting.
func CookieFromJson(value any) *Cookie {
	name, ok := field(value, "name").(string)
	if !ok {
		return nil
	}
	text, ok := field(value, "value").(string)
	if !ok {
		return nil
	}
	name = strings.TrimSpace(name)
	if name == "" {
		return nil
	}
	cookie := NewCookie(name, strings.TrimSpace(text), stringField(value, "domain"))
	if path, ok := field(value, "path").(string); ok && strings.TrimSpace(path) != "" {
		cookie.Path = path
	}
	if secure, ok := field(value, "secure").(bool); ok {
		cookie.Secure = secure
	}
	cookie.Expires = int64Field(field(value, "expires"))
	cookie.Discard, _ = field(value, "discard").(bool)
	return &cookie
}

// Json explicitly exports private state for encrypted server-side persistence.
func (cookie Cookie) Json() map[string]any {
	return map[string]any{"name": cookie.Name, "value": cookie.Value, "domain": cookie.Domain, "path": cookie.Path, "secure": cookie.Secure, "expires": optionalValue(cookie.Expires), "discard": cookie.Discard, "rest": map[string]any{}}
}

func optionalValue[T any](value *T) any {
	if value == nil {
		return nil
	}
	return *value
}

// IsUnexpired uses a strict expiry boundary and permits session cookies without timestamps.
func (cookie Cookie) IsUnexpired(now int64) bool {
	return cookie.Expires == nil || *cookie.Expires > now
}
func cloneCookies(cookies []Cookie) []Cookie {
	result := append([]Cookie{}, cookies...)
	for index, cookie := range result {
		if cookie.Expires != nil {
			expires := *cookie.Expires
			result[index].Expires = &expires
		}
	}
	return result
}
func field(value any, key string) any { object, _ := value.(map[string]any); return object[key] }
func stringField(value any, key string) string {
	result, _ := field(value, key).(string)
	return result
}
func textField(value any, key string) *string {
	text, ok := field(value, key).(string)
	if !ok {
		return nil
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return nil
	}
	return &text
}
func int64Field(value any) *int64 {
	switch number := value.(type) {
	case int64:
		return &number
	case int:
		parsed := int64(number)
		return &parsed
	case uint64:
		if number <= uint64(^uint64(0)>>1) {
			parsed := int64(number)
			return &parsed
		}
	case json.Number:
		parsed, err := domain.ParseNumber(number)
		if err == nil {
			integer, ok := parsed.AsInt64()
			if ok {
				return &integer
			}
		}
	case domain.Number:
		integer, ok := number.AsInt64()
		if ok {
			return &integer
		}
	}
	return nil
}
func pointer(value string) *string { return &value }
func cloneString(value *string) *string {
	if value == nil {
		return nil
	}
	return pointer(*value)
}
func cloneResult(value SearchResult) SearchResult {
	value.FileName = cloneString(value.FileName)
	value.DbName = cloneString(value.DbName)
	value.DbCode = cloneString(value.DbCode)
	value.DownloadUrl = cloneString(value.DownloadUrl)
	return value
}

// Transport owns login, cookie state, metadata inspection and bounded PDF downloads.
type Transport interface {
	StartQrLogin(context.Context) (QrLogin, error)
	PollQrLogin(context.Context, string, int64, float64) (string, error)
	SetLoginCookie(string)
	WarmUpFulltextSession(context.Context, string) (string, error)
	LoadCookies([]Cookie)
	Cookies() []Cookie
	HasUnexpiredCookie(string, int64) bool
	Search(context.Context, string, uint64) ([]SearchResult, error)
	InspectResultMetadata(context.Context, SearchResult) (ArticleCandidate, error)
	DownloadPdf(context.Context, string, *string, *string) (DownloadedPdf, error)
}
