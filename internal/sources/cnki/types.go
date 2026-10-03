// Package cnki preserves domestic NZKPT journal metadata and article access.
package cnki

import (
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

const NaviBase = "https://navi.cnki.net"
const KnsBase = "https://kns.cnki.net"

// Error retains domestic request, parser, fixture and permanent-missing classifications.
type Error struct {
	Kind, Message string
	Cause         error
}

func (failure *Error) Error() string {
	if failure.Kind == "PermanentArticleMissing" {
		return "domestic CNKI article is permanently unavailable"
	}
	if failure.Cause != nil {
		return failure.Cause.Error()
	}
	return failure.Message
}
func (failure *Error) Unwrap() error { return failure.Cause }

// HttpStatus extracts only an explicitly classified upstream HTTP status.
func (failure *Error) HttpStatus() (uint16, bool) {
	if failure.Kind != "Request" || !strings.HasPrefix(failure.Message, "domestic CNKI HTTP status ") {
		return 0, false
	}
	text := strings.TrimPrefix(failure.Message, "domestic CNKI HTTP status ")
	text = strings.TrimPrefix(text, "+")
	value, err := strconv.ParseUint(text, 10, 16)
	return uint16(value), err == nil
}

// JournalLocator retains ordered identity candidates and normalized match sets.
type JournalLocator struct {
	titles, issns                     []string
	normalizedTitles, normalizedIssns map[string]bool
}

// NewJournalLocator deduplicates title aliases and checks ISSN check digits.
func NewJournalLocator(titles, issns []string) JournalLocator {
	locator := JournalLocator{titles: []string{}, issns: []string{}, normalizedTitles: map[string]bool{}, normalizedIssns: map[string]bool{}}
	for _, title := range titles {
		if value := domain.NormalizeText(title); value != nil {
			key := domain.NormalizeBibliographicText(*value)
			if !locator.normalizedTitles[key] {
				locator.normalizedTitles[key] = true
				locator.titles = append(locator.titles, *value)
			}
		}
	}
	for _, issn := range issns {
		if value := domain.NormalizeIssn(issn); value != nil && !locator.normalizedIssns[*value] {
			locator.normalizedIssns[*value] = true
			locator.issns = append(locator.issns, *value)
		}
	}
	return locator
}

// Titles returns the normalized title queries in caller order.
func (locator JournalLocator) Titles() []string { return append([]string{}, locator.titles...) }

// Issns returns the valid, deduplicated ISSN queries in caller order.
func (locator JournalLocator) Issns() []string { return append([]string{}, locator.issns...) }

// IssueArticlePage retains zero-based page identity and the upstream count assertion.
type IssueArticlePage struct {
	Articles     []any  `json:"articles"`
	PageIndex    uint64 `json:"page_index"`
	ArticleCount uint64 `json:"article_count"`
	HasNextPage  bool   `json:"has_next_page"`
}
