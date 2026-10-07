// Package cfp acquires, preserves and refreshes original publisher announcements.
package cfp

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

const MaxPageBytes = 4 * 1024 * 1024
const MaxCaptureBytes = 24 * 1024 * 1024
const MaxDetailPages = 12

type Adapter string

const (
	SpringerCollections Adapter = "springer_collections"
	ElsevierCalls       Adapter = "elsevier_calls"
	KeaiCalls           Adapter = "keai_calls"
	SnapshotOnly        Adapter = "snapshot_only"
)

type UrlRule struct {
	Host       string `json:"host"`
	PathPrefix string `json:"pathPrefix"`
}
type SourceConfig struct {
	SourceKey              string    `json:"sourceKey"`
	CatalogIds             []string  `json:"catalogIds"`
	JournalTitle           string    `json:"journalTitle"`
	DiscoveryUrl           string    `json:"discoveryUrl"`
	Adapter                Adapter   `json:"adapter"`
	ConfigVersion          uint32    `json:"configVersion"`
	AllowedUrls            []UrlRule `json:"allowedUrls"`
	IdentityTexts          []string  `json:"identityTexts"`
	EmptyStatements        []string  `json:"emptyStatements"`
	CapabilityNote         *string   `json:"capabilityNote"`
	RetainsPreviousNotices bool      `json:"retainsPreviousNotices"`
}

// PermitsUrl enforces exact hosts and path-segment boundaries for every request and redirect.
func (config SourceConfig) PermitsUrl(location *whatwg.Url) bool {
	if !permittedUrlAuthority(location) {
		return false
	}
	for _, rule := range config.AllowedUrls {
		if permittedUrlRule(location, rule) {
			return true
		}
	}
	return false
}
func (config SourceConfig) CanRefresh() bool { return config.Adapter != SnapshotOnly }

// Registry returns independent packaged registrations, retaining snapshot-only capabilities.
func Registry() []SourceConfig {
	var sources []SourceConfig
	if err := json.Unmarshal(assets.Registry(), &sources); err != nil {
		panic(err)
	}
	for _, source := range sources {
		location, err := whatwg.NewParser().Parse(source.DiscoveryUrl)
		if err != nil || len(source.CatalogIds) == 0 || source.ConfigVersion == 0 || !source.PermitsUrl(location) {
			panic("invalid bundled CFP registration")
		}
	}
	return sources
}

// SourceError deliberately contains no captured text, credential or filesystem path.
type SourceError struct {
	Kind   string
	Status int
}

// Error reports acquisition failures without captured content or credentials.
func (failure SourceError) Error() string {
	switch failure.Kind {
	case "unsupported":
		return "Automatic discovery adapter is not yet available"
	case "deadline":
		return "Source acquisition deadline exceeded"
	case "disallowed_url":
		return "Source URL or address is outside the registered public boundary"
	case "request":
		return "Source request failed"
	case "http_status":
		return fmt.Sprintf("Source returned HTTP %d", failure.Status)
	case "cancelled":
		return "Source acquisition cancelled"
	default:
		return captureFailureMessage(failure.Kind)
	}
}

var (
	ErrUnsupported   = SourceError{Kind: "unsupported"}
	ErrDeadline      = SourceError{Kind: "deadline"}
	ErrDisallowedUrl = SourceError{Kind: "disallowed_url"}
	ErrRequest       = SourceError{Kind: "request"}
	ErrChallenge     = SourceError{Kind: "challenge"}
	ErrTooLarge      = SourceError{Kind: "too_large"}
	ErrEncoding      = SourceError{Kind: "encoding"}
	ErrContentType   = SourceError{Kind: "content_type"}
	ErrUnrecognized  = SourceError{Kind: "unrecognized"}
	ErrHelper        = SourceError{Kind: "helper"}
	ErrCancelled     = SourceError{Kind: "cancelled"}
)

type Document struct {
	FinalUrl string `json:"finalUrl"`
	Text     string `json:"text"`
	Format   string `json:"format"`
}
type Transport interface {
	Fetch(context.Context, SourceConfig, string, time.Time) (Document, error)
}
type Acquisition struct {
	Documents     []Document
	Sources       []domain.Source
	EmptyJournals []domain.EmptyJournal
}
type ParsedPage struct {
	Sources       []domain.Source
	EmptyJournals []domain.EmptyJournal
	DetailUrls    []string
	DetailTitles  map[string]string
}

// permittedUrlAuthority preserves HTTP schemes, absent credentials and permitted explicit ports.
func permittedUrlAuthority(location *whatwg.Url) bool {
	if location == nil || (location.Scheme() != "http" && location.Scheme() != "https") || location.Username() != "" || location.Password() != "" || location.Port() != "" && location.Port() != "80" && location.Port() != "443" {
		return false
	}
	return true
}

// permittedUrlRule matches exact hosts and original path-segment boundaries.
func permittedUrlRule(location *whatwg.Url, rule UrlRule) bool {
	return location.Hostname() == rule.Host && (rule.PathPrefix == "/" || location.Pathname() == rule.PathPrefix || strings.HasPrefix(strings.TrimPrefix(location.Pathname(), strings.TrimRight(rule.PathPrefix, "/")), "/") && strings.HasPrefix(location.Pathname(), strings.TrimRight(rule.PathPrefix, "/")))
}

// captureFailureMessage reports capture and extraction failures without including source content.
func captureFailureMessage(kind string) string {
	switch kind {
	case "challenge":
		return "Source returned an access challenge"
	case "too_large":
		return "Source capture exceeded its size or page bound"
	case "encoding":
		return "Source encoding could not be decoded faithfully"
	case "content_type":
		return "Unsupported source content type"
	case "unrecognized":
		return "Journal identity or complete CFP layout could not be verified"
	case "helper":
		return "Optional source helper is unavailable or failed"
	default:
		return "Source request failed"
	}
}
