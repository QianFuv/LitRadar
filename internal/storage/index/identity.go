// Package index implements canonical content identity and durable indexing storage.
package index

import (
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// ArticleIdentityKey identifies an immutable article independently of its source provider.
type ArticleIdentityKey struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
}

// ArticleIdentityResolution selects an existing alias owner or a deterministic new identity.
type ArticleIdentityResolution struct {
	ArticleId   int64              `json:"article_id"`
	IsExisting  bool               `json:"is_existing"`
	IdentityKey ArticleIdentityKey `json:"identity_key"`
}

// IdentityConflict rejects a bridge between distinct immutable article identities.
type IdentityConflict struct{ ArticleIds []int64 }

func (err *IdentityConflict) Error() string {
	values := make([]string, len(err.ArticleIds))
	for position, value := range err.ArticleIds {
		values[position] = strconv.FormatInt(value, 10)
	}
	return "article aliases resolve to multiple IDs: [" + strings.Join(values, ", ") + "]"
}

// JournalId derives a stable identifier from the immutable maintained catalog key.
func JournalId(catalogId string) int64 {
	return int64(identity.Stable(asciiLower(strings.TrimSpace(catalogId)), "journal:v1"))
}

// IssueIdentityValue retains the original year/label, date, then title precedence.
func IssueIdentityValue(journalId int64, issue domain.IssueDraft) *string {
	prefix := strconv.FormatInt(journalId, 10)
	volume, number := "", ""
	if issue.Volume != nil {
		volume = domain.NormalizeBibliographicLabel(*issue.Volume)
	}
	if issue.Number != nil {
		number = domain.NormalizeBibliographicLabel(*issue.Number)
	}
	if issue.PublicationYear != nil && (volume != "" || number != "") {
		return ptr(prefix + "|" + strconv.FormatInt(*issue.PublicationYear, 10) + "|" + volume + "|" + number)
	}
	if issue.Date != nil {
		if date := strings.TrimSpace(*issue.Date); date != "" {
			return ptr(prefix + "|date|" + date)
		}
	}
	if issue.Title != nil {
		if title := domain.NormalizeBibliographicText(*issue.Title); title != "" {
			return ptr(prefix + "|title|" + title)
		}
	}
	return nil
}

// IssueId returns no identifier when an issue has no sufficient bibliographic basis.
func IssueId(journalId int64, issue domain.IssueDraft) *int64 {
	if value := IssueIdentityValue(journalId, issue); value != nil {
		return ptr(int64(identity.Stable(*value, "issue:v1")))
	}
	return nil
}

// ArticleIdentityKeys returns aliases in strongest-to-weakest allocation order.
func ArticleIdentityKeys(article domain.ArticleDraft) []ArticleIdentityKey {
	keys := []ArticleIdentityKey{}
	if article.Doi != nil {
		if value := domain.NormalizeDoi(*article.Doi); value != nil {
			keys = append(keys, ArticleIdentityKey{"doi", *value})
		}
	}
	if article.Pmid != nil {
		if value := domain.NormalizePmid(*article.Pmid); value != nil {
			keys = append(keys, ArticleIdentityKey{"pmid", *value})
		}
	}
	if value := bibliographicFingerprint(article); value != nil {
		keys = append(keys, ArticleIdentityKey{"bibliographic", *value})
	}
	return keys
}

// ResolveArticleIdentity refuses fuzzy merging and retains the first matching canonical alias.
func ResolveArticleIdentity(article domain.ArticleDraft, aliases map[ArticleIdentityKey]int64) (ArticleIdentityResolution, error) {
	keys := ArticleIdentityKeys(article)
	if len(keys) == 0 {
		return ArticleIdentityResolution{}, errors.New("article has no canonical identity basis")
	}
	matched := []int64{}
	var first *ArticleIdentityKey
	for _, key := range keys {
		if owner, ok := aliases[key]; ok {
			if first == nil {
				first = ptr(key)
			}
			if !slices.Contains(matched, owner) {
				matched = append(matched, owner)
			}
		}
	}
	slices.Sort(matched)
	if len(matched) > 1 {
		return ArticleIdentityResolution{}, &IdentityConflict{matched}
	}
	if first != nil {
		return ArticleIdentityResolution{matched[0], true, *first}, nil
	}
	return ArticleIdentityResolution{int64(identity.Stable(keys[0].Value, "article:v1:"+keys[0].Kind)), false, keys[0]}, nil
}

func bibliographicFingerprint(article domain.ArticleDraft) *string {
	publication := ""
	if article.PublicationYear != nil {
		publication = strconv.FormatInt(*article.PublicationYear, 10)
	} else if article.Date != nil {
		publication = publicationYear(*article.Date)
	}
	if publication == "" {
		return nil
	}
	title := domain.NormalizeBibliographicText(article.Title)
	if title == "" {
		return nil
	}
	values := []string{"", "", ""}
	for position, value := range []*string{article.Volume, article.IssueNumber, article.StartPage} {
		if value != nil {
			values[position] = domain.NormalizeBibliographicLabel(*value)
		}
	}
	if values[0] == "" && values[1] == "" && values[2] == "" {
		return nil
	}
	return ptr(asciiLower(strings.TrimSpace(article.CatalogId)) + "|" + title + "|" + publication + "|" + strings.Join(values, "|"))
}

func publicationYear(value string) string {
	year, _, _ := strings.Cut(value, "-")
	year = strings.TrimSpace(year)
	if len(year) != 4 {
		return ""
	}
	for _, character := range year {
		if character < '0' || character > '9' {
			return ""
		}
	}
	return year
}

// MergeArticleDrafts applies commutative enrichment rules before identity resolution.
func MergeArticleDrafts(left, right domain.ArticleDraft) (domain.ArticleDraft, error) {
	return mergeDrafts(left, right, false)
}

// MergeResolvedArticleDrafts refreshes publication metadata after aliases establish one identity.
func MergeResolvedArticleDrafts(left, right domain.ArticleDraft) (domain.ArticleDraft, error) {
	merged, err := mergeDrafts(left, right, true)
	if err != nil {
		return merged, err
	}
	preferred, fallback := right, left
	if left.InPress != nil && !*left.InPress && right.InPress != nil && *right.InPress {
		preferred, fallback = left, right
	}
	merged.Date = resolvedPublicationDate(preferred, fallback)
	merged.PublicationYear = resolvedPublicationYear(merged.Date, preferred, fallback)
	merged.IssueTitle = firstValue(preferred.IssueTitle, fallback.IssueTitle)
	merged.Volume = firstValue(preferred.Volume, fallback.Volume)
	merged.IssueNumber = firstValue(preferred.IssueNumber, fallback.IssueNumber)
	merged.StartPage = firstValue(preferred.StartPage, fallback.StartPage)
	merged.EndPage = firstValue(preferred.EndPage, fallback.EndPage)
	return merged, nil
}

func mergeDrafts(left, right domain.ArticleDraft, canMergeDois bool) (domain.ArticleDraft, error) {
	if err := validateDraftMerge(left, right, canMergeDois); err != nil {
		return domain.ArticleDraft{}, err
	}
	doi := orderedText(left.Doi, right.Doi)
	year := firstValue(left.PublicationYear, right.PublicationYear)
	if year != nil && right.PublicationYear != nil && *right.PublicationYear < *year {
		year = copyValue(right.PublicationYear)
	}
	authors := selectMergedAuthors(left.Authors, right.Authors)
	retractions := append(append([]string{}, left.RetractionDois...), right.RetractionDois...)
	slices.Sort(retractions)
	retractions = slices.Compact(retractions)
	return domain.ArticleDraft{CatalogId: left.CatalogId, Title: richerText(left.Title, right.Title), PublicationYear: year, Date: richerOptional(left.Date, right.Date), IssueTitle: richerOptional(left.IssueTitle, right.IssueTitle), Volume: orderedText(left.Volume, right.Volume), IssueNumber: orderedText(left.IssueNumber, right.IssueNumber), Authors: append([]domain.ArticleAuthorDraft{}, authors...), StartPage: orderedText(left.StartPage, right.StartPage), EndPage: orderedText(left.EndPage, right.EndPage), AbstractText: richerOptional(left.AbstractText, right.AbstractText), Doi: doi, Pmid: firstValue(left.Pmid, right.Pmid), OpenAccess: mergeBool(left.OpenAccess, right.OpenAccess, true), InPress: mergeBool(left.InPress, right.InPress, false), RetractionDois: retractions}, nil
}

func ptr[T any](value T) *T { return &value }
func copyValue[T any](value *T) *T {
	if value == nil {
		return nil
	}
	return ptr(*value)
}
func firstValue[T any](first, second *T) *T {
	if first != nil {
		return copyValue(first)
	}
	return copyValue(second)
}
func orderedText(first, second *string) *string {
	if first != nil && second != nil {
		return ptr(min(*first, *second))
	}
	return firstValue(first, second)
}
func richerText(first, second string) string {
	if len(first) > len(second) {
		return first
	}
	if len(second) > len(first) {
		return second
	}
	return min(first, second)
}
func richerOptional(first, second *string) *string {
	if first != nil && second != nil {
		return ptr(richerText(*first, *second))
	}
	return firstValue(first, second)
}
func mergeBool(first, second *bool, isTruePreferred bool) *bool {
	if first == nil || second == nil {
		return firstValue(first, second)
	}
	if isTruePreferred {
		return ptr(*first || *second)
	}
	return ptr(*first && *second)
}
func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}

func resolvedPublicationDate(preferred, fallback domain.ArticleDraft) *string {
	date := copyValue(preferred.Date)
	if date == nil && fallback.Date != nil {
		isCompatible := preferred.PublicationYear == nil
		if !isCompatible {
			year, err := strconv.ParseInt(publicationYear(*fallback.Date), 10, 64)
			isCompatible = err == nil && year == *preferred.PublicationYear
		}
		if isCompatible {
			date = copyValue(fallback.Date)
		}
	}
	return date
}

func resolvedPublicationYear(date *string, preferred, fallback domain.ArticleDraft) *int64 {
	var yearValue *int64
	if date != nil {
		if year, err := strconv.ParseInt(publicationYear(*date), 10, 64); err == nil {
			yearValue = &year
		}
	}
	if yearValue == nil {
		yearValue = firstValue(preferred.PublicationYear, fallback.PublicationYear)
	}
	return yearValue
}

func validateDraftMerge(left, right domain.ArticleDraft, canMergeDois bool) error {
	if left.CatalogId != right.CatalogId {
		return errors.New("article drafts use different catalog IDs")
	}
	if !canMergeDois && left.Doi != nil && right.Doi != nil && *left.Doi != *right.Doi {
		return fmt.Errorf("article drafts contain conflicting DOI values")
	}
	if left.Pmid != nil && right.Pmid != nil && *left.Pmid != *right.Pmid {
		return fmt.Errorf("article drafts contain conflicting PMID values")
	}
	return nil
}

func selectMergedAuthors(left, right []domain.ArticleAuthorDraft) []domain.ArticleAuthorDraft {
	authors := left
	if len(right) > len(authors) || len(right) == len(authors) && slices.CompareFunc(right, authors, func(first, second domain.ArticleAuthorDraft) int {
		return strings.Compare(first.DisplayName, second.DisplayName)
	}) < 0 {
		authors = right
	}
	return authors
}
