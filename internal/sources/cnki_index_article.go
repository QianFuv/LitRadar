package sources

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

var cnkiPersonalMarker = regexp.MustCompile(`^(?:[0-9]+[.]?)?([\p{Han}·]{2,8})(?:[0-9]+[a-c]?|[a-c]|[①-⑳†‡*]+)?$`)

// cnkiAuthors retains ordered normalized names and strips personal markers only outside organization names.
func cnkiAuthors(value *string) []domain.ArticleAuthorDraft {
	authors := []domain.ArticleAuthorDraft{}
	if value == nil {
		return authors
	}
	for _, part := range strings.FieldsFunc(*value, func(character rune) bool { return strings.ContainsRune(";；,，", character) }) {
		name := domain.NormalizeText(part)
		if name == nil {
			continue
		}
		if isCnkiAuthorMarker(*name) {
			continue
		}
		isOrganization := isCnkiAuthorOrganization(*name)
		if !isOrganization {
			if matched := cnkiPersonalMarker.FindStringSubmatch(*name); matched != nil {
				name = &matched[1]
			}
		}
		authors = append(authors, domain.ArticleAuthorDraft{DisplayName: *name})
	}
	return authors
}

func cnkiInteger(value any) *int64 {
	var number domain.Number
	var err error
	switch value := value.(type) {
	case domain.Number:
		number = value
	case json.Number:
		number, err = domain.ParseNumber(value)
	case int:
		result := int64(value)
		return &result
	case int64:
		return &value
	case uint64:
		number, err = domain.ParseNumber(json.Number(strconv.FormatUint(value, 10)))
	default:
		return nil
	}
	if err != nil {
		return nil
	}
	if integer, ok := number.AsInt64(); ok {
		return &integer
	}
	return nil
}

func cnkiIssueDraft(catalog domain.JournalCatalogEntry, payload any) domain.IssueDraft {
	year := cnkiInteger(providerField(payload, "year"))
	if year == nil {
		if text := providerText(providerField(payload, "year")); text != nil {
			if parsed, err := strconv.ParseInt(*text, 10, 64); err == nil {
				year = &parsed
			}
		}
	}
	number := providerText(providerField(payload, "number"))
	var date *string
	if year != nil {
		date = new(fmt.Sprintf("%04d", *year))
		if number != nil {
			if month, err := strconv.ParseUint(strings.TrimPrefix(*number, "+"), 10, 8); err == nil && month >= 1 && month <= 12 {
				date = new(fmt.Sprintf("%04d-%02d", *year, month))
			}
		}
	}
	return domain.IssueDraft{CatalogId: catalog.CatalogId, PublicationYear: year, Title: providerText(providerField(payload, "title")), Volume: providerText(providerField(payload, "volume")), Number: number, Date: date}
}

func firstCnkiText(values ...*string) *string {
	for _, value := range values {
		if value != nil {
			return value
		}
	}
	return nil
}

func cnkiBool(value any) *bool {
	if flag, ok := value.(bool); ok {
		return &flag
	}
	if integer := cnkiInteger(value); integer != nil {
		return new(*integer != 0)
	}
	if text, ok := value.(string); ok {
		switch asciiLowerSource(strings.TrimSpace(text)) {
		case "true", "1", "yes":
			return new(true)
		case "false", "0", "no":
			return new(false)
		}
	}
	return nil
}

func cnkiArticleDraft(catalog domain.JournalCatalogEntry, issue domain.IssueDraft, summary, detail any) *domain.ArticleDraft {
	title := firstCnkiText(providerText(providerField(detail, "title")), providerText(providerField(summary, "title")))
	if title == nil {
		return nil
	}
	date := firstCnkiText(providerText(providerField(detail, "online_release_date")), providerText(providerField(detail, "date")), providerText(providerField(detail, "publication_date")), providerText(providerField(summary, "date")), issue.Date)
	year := issue.PublicationYear
	if date != nil && len(*date) >= 4 && utf8.ValidString((*date)[:4]) {
		if parsed, err := strconv.ParseInt((*date)[:4], 10, 64); err == nil {
			year = &parsed
		}
	}
	start, end := domain.SplitPages(firstCnkiText(providerText(providerField(detail, "pages")), providerText(providerField(summary, "pages"))))
	authors := cnkiAuthors(firstCnkiText(providerText(providerField(detail, "authors")), providerText(providerField(summary, "authors"))))
	var doi, pmid *string
	if text := providerText(providerField(detail, "doi")); text != nil {
		doi = domain.NormalizeDoi(*text)
	}
	if text := providerText(providerField(detail, "pmid")); text != nil {
		pmid = domain.NormalizePmid(*text)
	}
	retractions := []string{}
	if text := providerText(providerField(detail, "retraction_doi")); text != nil {
		if normalized := domain.NormalizeDoi(*text); normalized != nil {
			retractions = append(retractions, *normalized)
		}
	}
	return domain.CanonicalArticle(domain.ArticleDraft{CatalogId: catalog.CatalogId, Title: *title, PublicationYear: year, Date: date, IssueTitle: issue.Title, Volume: issue.Volume, IssueNumber: issue.Number, Authors: authors, StartPage: start, EndPage: end, AbstractText: providerText(providerField(detail, "abstract")), Doi: doi, Pmid: pmid, OpenAccess: cnkiBool(providerField(detail, "open_access")), InPress: new(false), RetractionDois: retractions})
}

func cnkiLacksAuthorsAndDoi(summary, detail any) bool {
	if len(cnkiAuthors(firstCnkiText(providerText(providerField(detail, "authors")), providerText(providerField(summary, "authors"))))) > 0 {
		return false
	}
	text := providerText(providerField(detail, "doi"))
	return text == nil || domain.NormalizeDoi(*text) == nil
}

// isCnkiAuthorMarker recognizes only the original digit, circled-digit and suffix marker characters.
func isCnkiAuthorMarker(name string) bool {
	isMarker := true
	for _, character := range name {
		if !(character >= '0' && character <= '9' || character >= '①' && character <= '⑳' || strings.ContainsRune("abc†‡*", character)) {
			isMarker = false
			break
		}
	}
	return isMarker
}

// isCnkiAuthorOrganization preserves personal-marker text when an organization word is present.
func isCnkiAuthorOrganization(name string) bool {
	isOrganization := false
	for _, word := range []string{"组", "委员会", "研究院", "科学院", "研究所", "大学", "中心"} {
		isOrganization = isOrganization || strings.Contains(name, word)
	}
	return isOrganization
}
