package scholarly

import (
	"slices"
	"sort"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func firstText(value any) *string {
	if values, ok := value.([]any); ok {
		for _, value := range values {
			if text := jsonText(value); text != nil {
				return text
			}
		}
		return nil
	}
	return jsonText(value)
}

func textDoi(value any) *string {
	if text := jsonText(value); text != nil {
		return domain.NormalizeDoi(*text)
	}
	return nil
}
func textPmid(value any) *string {
	if text := jsonText(value); text != nil {
		return domain.NormalizePmid(*text)
	}
	return nil
}
func strictBool(value any) *bool {
	if value, ok := value.(bool); ok {
		return &value
	}
	return nil
}
func yearFromDate(date *string) *int64 {
	if date != nil && len(*date) >= 4 {
		if year, err := strconv.ParseInt((*date)[:4], 10, 64); err == nil {
			return &year
		}
	}
	return nil
}

func crossrefAuthors(value any) []domain.ArticleAuthorDraft {
	authors := []domain.ArticleAuthorDraft{}
	for _, author := range array(value) {
		parts := []string{}
		for _, key := range []string{"given", "family"} {
			if text := jsonText(field(author, key)); text != nil {
				parts = append(parts, *text)
			}
		}
		if name := domain.NormalizeText(strings.Join(parts, " ")); name != nil {
			authors = append(authors, domain.ArticleAuthorDraft{DisplayName: *name})
		}
	}
	return authors
}

func stripMarkup(value string) *string {
	var output strings.Builder
	isInside := false
	for _, character := range value {
		switch character {
		case '<':
			isInside = true
		case '>':
			isInside = false
		default:
			if !isInside {
				output.WriteRune(character)
			}
		}
	}
	return domain.NormalizeText(output.String())
}

func openAlexAbstract(value any) *string {
	object, ok := field(value, "abstract_inverted_index").(map[string]any)
	if !ok {
		return nil
	}
	keys := make([]string, 0, len(object))
	for key := range object {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	type wordPosition struct {
		position int64
		word     string
	}
	positions := []wordPosition{}
	for _, word := range keys {
		indices, ok := object[word].([]any)
		if !ok {
			return nil
		}
		for _, index := range indices {
			position := signedNumber(index)
			if position == nil {
				return nil
			}
			positions = append(positions, wordPosition{*position, word})
		}
	}
	sort.SliceStable(positions, func(first, second int) bool { return positions[first].position < positions[second].position })
	words := make([]string, 0, len(positions))
	for _, entry := range positions {
		words = append(words, entry.word)
	}
	return domain.NormalizeText(strings.Join(words, " "))
}

func retractionDois(value any) []string {
	seen := map[string]bool{}
	result := []string{}
	for _, item := range array(value) {
		kind := jsonText(field(item, "type"))
		if kind == nil || !strings.EqualFold(*kind, "retraction") {
			continue
		}
		if doi := textDoi(field(item, "DOI")); doi != nil && !seen[*doi] {
			seen[*doi] = true
			result = append(result, *doi)
		}
	}
	slices.Sort(result)
	return result
}

func crossrefArticle(catalog domain.JournalCatalogEntry, work, openAlex, semanticScholar any) *domain.ArticleDraft {
	doi := textDoi(field(work, "DOI"))
	title := crossrefArticleTitle(work, openAlex, semanticScholar, doi)
	date := CrossrefDate(work)
	start, end := domain.SplitPages(jsonText(field(work, "page")))
	var abstract *string
	if text := jsonText(field(work, "abstract")); text != nil {
		abstract = stripMarkup(*text)
	}
	if abstract == nil {
		abstract = openAlexAbstract(openAlex)
	}
	if abstract == nil {
		abstract = jsonText(field(semanticScholar, "abstract"))
	}
	openAccess := strictBool(field(semanticScholar, "isOpenAccess"))
	if openAccess == nil && openAlex != nil {
		openAccess = new(field(openAlex, "best_oa_location") != nil)
	}
	object, _ := work.(map[string]any)
	_, hasIssue := object["issue"]
	return domain.CanonicalArticle(domain.ArticleDraft{CatalogId: catalog.CatalogId, Title: *title, PublicationYear: yearFromDate(date), Date: date, Volume: jsonText(field(work, "volume")), IssueNumber: jsonText(field(work, "issue")), Authors: crossrefAuthors(field(work, "author")), StartPage: start, EndPage: end, AbstractText: abstract, Doi: doi, Pmid: textPmid(field(work, "PMID")), OpenAccess: openAccess, InPress: new(!hasIssue), RetractionDois: retractionDois(field(work, "updated-by"))})
}

func openAlexArticle(catalog domain.JournalCatalogEntry, work any) *domain.ArticleDraft {
	title := jsonText(field(work, "display_name"))
	if title == nil {
		title = jsonText(field(work, "title"))
	}
	if title == nil {
		return nil
	}
	date := jsonText(field(work, "publication_date"))
	year := signedNumber(field(work, "publication_year"))
	if year == nil {
		year = yearFromDate(date)
	}
	biblio := field(work, "biblio")
	authors := []domain.ArticleAuthorDraft{}
	for _, entry := range array(field(work, "authorships")) {
		if name := jsonText(field(field(entry, "author"), "display_name")); name != nil {
			authors = append(authors, domain.ArticleAuthorDraft{DisplayName: *name})
		}
	}
	return domain.CanonicalArticle(domain.ArticleDraft{CatalogId: catalog.CatalogId, Title: *title, PublicationYear: year, Date: date, Volume: jsonText(field(biblio, "volume")), IssueNumber: jsonText(field(biblio, "issue")), StartPage: jsonText(field(biblio, "first_page")), EndPage: jsonText(field(biblio, "last_page")), Authors: authors, AbstractText: openAlexAbstract(work), Doi: textDoi(field(work, "doi")), OpenAccess: strictBool(field(field(work, "open_access"), "is_oa")), InPress: new(false), RetractionDois: []string{}})
}

// crossrefArticleTitle uses enrichment titles only for matching normalized DOI identities.
func crossrefArticleTitle(work, openAlex, semanticScholar any, doi *string) *string {
	title := firstText(field(work, "title"))
	if title == nil && doi != nil {
		if hasMatchingArticleDoi(field(openAlex, "doi"), *doi) {
			title = jsonText(field(openAlex, "display_name"))
			if title == nil {
				title = jsonText(field(openAlex, "title"))
			}
		}
	}
	if title == nil && doi != nil {
		if hasMatchingArticleDoi(field(field(semanticScholar, "externalIds"), "DOI"), *doi) {
			title = jsonText(field(semanticScholar, "title"))
		}
	}
	if title == nil {
		title = new("")
	}
	return title
}

// hasMatchingArticleDoi requires a textual DOI matching the primary normalized identity.
func hasMatchingArticleDoi(value any, doi string) bool {
	observed, ok := value.(string)
	if !ok {
		return false
	}
	normalized := domain.NormalizeDoi(observed)
	return normalized != nil && *normalized == doi
}
