package sources

import (
	"encoding/json"
	"slices"
	"strings"
)

// CanonicalArticle normalizes source content while requiring an external or bibliographic identity.
func CanonicalArticle(article ArticleDraft) *ArticleDraft {
	article.PublicationYear = copyCanonicalField(article.PublicationYear)
	article.Doi = copyCanonicalField(article.Doi)
	article.Pmid = copyCanonicalField(article.Pmid)
	article.OpenAccess = copyCanonicalField(article.OpenAccess)
	article.InPress = copyCanonicalField(article.InPress)
	if title := NormalizeText(article.Title); title != nil {
		article.Title = *title
	} else if article.Doi != nil {
		doi := NormalizeDoi(*article.Doi)
		if doi == nil || *doi != *article.Doi {
			return nil
		}
		article.Title = ""
	} else {
		return nil
	}
	if article.Date != nil {
		date := NormalizeDate(*article.Date)
		article.Date = nil
		if date != nil {
			article.Date = &date.Value
		}
	}
	for _, field := range []**string{&article.IssueTitle, &article.Volume, &article.IssueNumber, &article.StartPage, &article.EndPage, &article.AbstractText} {
		if *field != nil {
			*field = NormalizeText(**field)
		}
	}
	authors := make([]ArticleAuthorDraft, 0, len(article.Authors))
	for _, author := range article.Authors {
		if name := NormalizeText(author.DisplayName); name != nil {
			authors = append(authors, ArticleAuthorDraft{DisplayName: *name})
		}
	}
	article.Authors = authors
	seen := map[string]bool{}
	articleRetractions := []string{}
	for _, value := range article.RetractionDois {
		if doi := NormalizeDoi(value); doi != nil && !seen[*doi] {
			seen[*doi] = true
			articleRetractions = append(articleRetractions, *doi)
		}
	}
	slices.Sort(articleRetractions)
	article.RetractionDois = articleRetractions
	if article.Doi == nil && article.Pmid == nil && !(article.PublicationYear != nil && (article.Volume != nil || article.IssueNumber != nil || article.StartPage != nil)) {
		return nil
	}
	return &article
}

// SplitPages preserves the source separator precedence and independently normalizes endpoints.
func SplitPages(value *string) (*string, *string) {
	if value == nil {
		return nil, nil
	}
	normalized := NormalizeText(*value)
	if normalized == nil {
		return nil, nil
	}
	for _, separator := range []string{"-", "–", "—"} {
		if first, last, ok := strings.Cut(*normalized, separator); ok {
			return NormalizeText(first), NormalizeText(last)
		}
	}
	return normalized, nil
}

// CatalogIssns preserves maintained order, appending absent print and electronic identities.
func CatalogIssns(catalog JournalCatalogEntry) []string {
	values := append([]string{}, catalog.AllIssns...)
	for _, value := range []*string{catalog.Issn, catalog.Eissn} {
		if value != nil && !slices.Contains(values, *value) {
			values = append(values, *value)
		}
	}
	return values
}

// IssueFromArticle extracts only an issue with a sufficient canonical boundary.
func IssueFromArticle(article ArticleDraft) *IssueDraft {
	if !(article.PublicationYear != nil && (article.Volume != nil || article.IssueNumber != nil) || article.Date != nil || article.IssueTitle != nil) {
		return nil
	}
	return &IssueDraft{CatalogId: article.CatalogId, PublicationYear: copyCanonicalField(article.PublicationYear), Title: copyCanonicalField(article.IssueTitle), Volume: copyCanonicalField(article.Volume), Number: copyCanonicalField(article.IssueNumber), Date: copyCanonicalField(article.Date)}
}

func copyCanonicalField[T any](value *T) *T {
	if value == nil {
		return nil
	}
	copy := *value
	return &copy
}

// BatchFromArticles preserves first occurrence order while deduplicating observed issue identities.
func BatchFromArticles(catalog JournalCatalogEntry, articles []ArticleDraft, progress ProviderProgress) ProviderBatch {
	issues := []IssueDraft{}
	seen := map[string]bool{}
	for _, article := range articles {
		if issue := IssueFromArticle(article); issue != nil {
			key, _ := json.Marshal([]any{issue.PublicationYear, issue.Volume, issue.Number, issue.Date, issue.Title})
			if !seen[string(key)] {
				seen[string(key)] = true
				issues = append(issues, *issue)
			}
		}
	}
	if articles == nil {
		articles = []ArticleDraft{}
	}
	return ProviderBatch{CatalogId: catalog.CatalogId, Journal: JournalDraft{CatalogId: catalog.CatalogId, ObservedTitle: &catalog.Title, ObservedIssns: CatalogIssns(catalog), ObservedTitleAliases: []string{}}, Issues: issues, Articles: articles, Progress: progress}
}
