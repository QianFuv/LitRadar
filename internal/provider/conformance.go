package provider

import (
	"context"
	"errors"
	"slices"
	"strconv"
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// ValidateIndexProviderFixture runs a provider with bounded opaque state and validates its complete batch.
func ValidateIndexProviderFixture(ctx context.Context, implementation IndexContent, catalog domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
	if err := ValidateOpaqueState(fetch.CommittedAnchor, "provider anchor"); err != nil {
		return domain.ProviderBatch{}, err
	}
	if err := ValidateOpaqueState(fetch.TraversalCheckpoint, "provider checkpoint"); err != nil {
		return domain.ProviderBatch{}, err
	}
	batch, err := implementation.Fetch(ctx, catalog, fetch)
	if err != nil {
		return domain.ProviderBatch{}, violation("index provider fixture failed with " + string(failureKind(err)))
	}
	if err := ValidateProviderBatch(catalog, batch); err != nil {
		return domain.ProviderBatch{}, err
	}
	return batch, nil
}

// ValidateAbstractProviderFixture checks local identity before resolving and validating a redirect.
func ValidateAbstractProviderFixture(ctx context.Context, implementation ArticleAbstract, article domain.ArticleLocator, access domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	if err := ValidateArticleLocator(article); err != nil {
		return domain.ArticleRedirect{}, err
	}
	if !implementation.SupportsAbstract(article) {
		return domain.ArticleRedirect{}, violation("abstract provider fixture does not support its locator")
	}
	redirect, err := implementation.ResolveAbstract(ctx, article, access)
	if err != nil {
		return domain.ArticleRedirect{}, violation("abstract provider fixture failed with " + string(failureKind(err)))
	}
	if err := ValidateArticleRedirect(redirect); err != nil {
		return domain.ArticleRedirect{}, err
	}
	return redirect, nil
}

// ValidateFullTextProviderFixture validates support, failure classification and bounded returned content.
func ValidateFullTextProviderFixture(ctx context.Context, implementation ArticleFullText, article domain.ArticleLocator, access domain.ArticleAccessContext, maximum int) (domain.ArticleFullTextResolution, error) {
	if err := ValidateArticleLocator(article); err != nil {
		return domain.ArticleFullTextResolution{}, err
	}
	if !implementation.SupportsFullText(article) {
		return domain.ArticleFullTextResolution{}, violation("full-text provider fixture does not support its locator")
	}
	resolution, err := implementation.ResolveFullText(ctx, article, access)
	if err != nil {
		return domain.ArticleFullTextResolution{}, violation("full-text provider fixture failed with " + string(failureKind(err)))
	}
	if err := ValidateFullTextResolution(resolution, maximum); err != nil {
		return domain.ArticleFullTextResolution{}, err
	}
	return resolution, nil
}

// ContractViolation reports a provider contract failure without raw payload content.
type ContractViolation struct{ Message string }

func (err *ContractViolation) Error() string { return err.Message }
func violation(message string) error         { return &ContractViolation{message} }

// ValidateCatalogEntry verifies maintained identity, canonical aliases and rankings in source order.
func ValidateCatalogEntry(entry domain.JournalCatalogEntry) error {
	if err := validateCatalogId(entry.CatalogId); err != nil {
		return err
	}
	ids := []string{entry.CatalogId}
	for _, alias := range entry.CatalogAliases {
		if err := validateCatalogId(alias); err != nil {
			return err
		}
		if slices.Contains(ids, alias) {
			return violation("catalog aliases must be distinct from catalog_id and each other")
		}
		ids = append(ids, alias)
	}
	if err := requireText(entry.Title, "catalog title"); err != nil {
		return err
	}
	if entry.Issn != nil {
		if err := requireIssn(*entry.Issn, "print ISSN"); err != nil {
			return err
		}
	}
	if entry.Eissn != nil {
		if err := requireIssn(*entry.Eissn, "electronic ISSN"); err != nil {
			return err
		}
	}
	issns := make([]string, 0, len(entry.AllIssns))
	for _, value := range entry.AllIssns {
		if err := requireIssn(value, "catalog ISSN"); err != nil {
			return err
		}
		if slices.Contains(issns, value) {
			return violation("catalog ISSNs must be unique")
		}
		issns = append(issns, value)
	}
	for _, value := range []*string{entry.Issn, entry.Eissn} {
		if value != nil && !slices.Contains(issns, *value) {
			return violation("primary catalog ISSNs must appear in all_issns")
		}
	}
	title := domain.NormalizeBibliographicText(entry.Title)
	aliases := make([]string, 0, len(entry.TitleAliases))
	for _, alias := range entry.TitleAliases {
		if err := requireText(alias, "title alias"); err != nil {
			return err
		}
		normalized := domain.NormalizeBibliographicText(alias)
		if normalized == title || slices.Contains(aliases, normalized) {
			return violation("catalog title aliases must be distinct")
		}
		aliases = append(aliases, normalized)
	}
	if err := optionalText(entry.Area, "catalog area"); err != nil {
		return err
	}
	for _, field := range []optionalField{{entry.Rankings.UtdRank, "UTD rank"}, {entry.Rankings.UtdRating, "UTD rating"}, {entry.Rankings.AbsRank, "ABS rank"}, {entry.Rankings.AbsRating, "ABS rating"}, {entry.Rankings.FmsRank, "FMS rank"}, {entry.Rankings.FmsRating, "FMS rating"}, {entry.Rankings.FmscnRank, "FMS China rank"}, {entry.Rankings.FmscnRating, "FMS China rating"}} {
		if err := optionalText(field.value, field.name); err != nil {
			return err
		}
	}
	return nil
}

// ValidateArticleLocator validates local metadata before any provider supports/resolve calls.
func ValidateArticleLocator(article domain.ArticleLocator) error {
	if article.ArticleId <= 0 {
		return violation("article locator must contain a positive internal ID")
	}
	if err := validateCatalogId(article.CatalogId); err != nil {
		return err
	}
	if err := requireText(article.JournalTitle, "locator journal title"); err != nil {
		return err
	}
	if err := requireArticleTitle(article.Title, article.Doi, "locator article title"); err != nil {
		return err
	}
	if err := optionalYear(article.PublicationYear, "locator publication year"); err != nil {
		return err
	}
	if err := optionalDate(article.Date); err != nil {
		return err
	}
	if !yearAgrees(article.PublicationYear, article.Date) {
		return violation("locator publication year and date must agree")
	}
	issns := make([]string, 0, len(article.JournalIssns))
	for _, issn := range article.JournalIssns {
		if err := requireIssn(issn, "locator journal ISSN"); err != nil {
			return err
		}
		if slices.Contains(issns, issn) {
			return violation("locator journal ISSNs must be unique")
		}
		issns = append(issns, issn)
	}
	for _, author := range article.Authors {
		if err := requireText(author, "locator author"); err != nil {
			return err
		}
	}
	for _, field := range []optionalField{{article.Volume, "locator volume"}, {article.IssueNumber, "locator issue number"}, {article.StartPage, "locator start page"}, {article.EndPage, "locator end page"}} {
		if err := optionalText(field.value, field.name); err != nil {
			return err
		}
	}
	if err := optionalDoi(article.Doi, "locator DOI"); err != nil {
		return err
	}
	if article.Pmid != nil && !sameOptional(domain.NormalizePmid(*article.Pmid), *article.Pmid) {
		return violation("locator PMID must use canonical digits")
	}
	return nil
}

// ValidateArticleRedirect preserves the original bounded HTTP(S) shape validation.
// Provider-specific allowlists apply separately before transmitting or returning a destination.
func ValidateArticleRedirect(redirect domain.ArticleRedirect) error {
	location := redirect.Location
	if len(location) > 8192 || location != strings.TrimSpace(location) || strings.ContainsFunc(location, unicode.IsControl) {
		return violation("article redirect must be a bounded canonical HTTP(S) URL")
	}
	remainder, hasScheme := strings.CutPrefix(location, "https://")
	if !hasScheme {
		remainder, hasScheme = strings.CutPrefix(location, "http://")
	}
	if !hasScheme {
		return violation("article redirect must use HTTP(S)")
	}
	authority := remainder
	if index := strings.IndexAny(authority, "/?#"); index >= 0 {
		authority = authority[:index]
	}
	if authority == "" || strings.Contains(authority, "@") || strings.ContainsFunc(authority, unicode.IsSpace) {
		return violation("article redirect authority is not safe")
	}
	return nil
}

// ValidateFullTextResolution checks the document size, canonical media type and download basename.
func ValidateFullTextResolution(resolution domain.ArticleFullTextResolution, maximum int) error {
	if (resolution.Redirect == nil) == (resolution.Document == nil) {
		return violation("full-text resolution must contain exactly one result")
	}
	if resolution.Redirect != nil {
		return ValidateArticleRedirect(*resolution.Redirect)
	}
	document := resolution.Document
	if maximum <= 0 || len(document.Bytes) == 0 || len(document.Bytes) > maximum {
		return violation("full-text document exceeds the configured size contract")
	}
	if err := requireText(document.ContentType, "full-text content type"); err != nil {
		return err
	}
	if document.ContentType != asciiLower(document.ContentType) || !strings.Contains(document.ContentType, "/") || strings.ContainsFunc(document.ContentType, unicode.IsSpace) {
		return violation("full-text content type must be canonical")
	}
	if document.Filename != nil {
		if err := requireText(*document.Filename, "full-text filename"); err != nil {
			return err
		}
		if len(*document.Filename) > 255 || strings.ContainsAny(*document.Filename, "/\\") || strings.ContainsFunc(*document.Filename, unicode.IsControl) {
			return violation("full-text filename must be a safe basename")
		}
	}
	return nil
}

// ValidateProviderBatch checks every canonical item before progress can be committed.
func ValidateProviderBatch(catalog domain.JournalCatalogEntry, batch domain.ProviderBatch) error {
	if err := ValidateCatalogEntry(catalog); err != nil {
		return err
	}
	if batch.CatalogId != catalog.CatalogId || batch.Journal.CatalogId != catalog.CatalogId {
		return violation("provider batch must echo the requested catalog_id")
	}
	if err := validateJournal(catalog, batch.Journal); err != nil {
		return err
	}
	for _, issue := range batch.Issues {
		if issue.CatalogId != catalog.CatalogId {
			return violation("issue must echo the requested catalog_id")
		}
		if err := validateIssue(issue); err != nil {
			return err
		}
	}
	for _, article := range batch.Articles {
		if err := validateArticle(catalog, article); err != nil {
			return err
		}
	}
	switch batch.Progress.State {
	case domain.Continue:
		if batch.Progress.Checkpoint == nil || batch.Progress.NextAnchor != nil {
			return violation("provider continuation must contain only a checkpoint")
		}
		return ValidateOpaqueState(batch.Progress.Checkpoint, "provider checkpoint")
	case domain.Complete:
		if batch.Progress.Checkpoint != nil {
			return violation("completed provider progress must not contain a checkpoint")
		}
		return ValidateOpaqueState(batch.Progress.NextAnchor, "provider anchor")
	default:
		return violation("invalid provider progress state")
	}
}

// ValidateOpaqueState accepts absent state and bounds each present opaque value by UTF-8 bytes.
func ValidateOpaqueState(value *string, field string) error {
	if value == nil {
		return nil
	}
	if *value == "" {
		return violation(field + " must not be empty when present")
	}
	if len(*value) > 65536 {
		return violation(field + " exceeds the contract limit")
	}
	return nil
}

func validateJournal(catalog domain.JournalCatalogEntry, journal domain.JournalDraft) error {
	isKnown := func(value string) bool {
		normalized := domain.NormalizeBibliographicText(value)
		if normalized == domain.NormalizeBibliographicText(catalog.Title) {
			return true
		}
		for _, alias := range catalog.TitleAliases {
			if normalized == domain.NormalizeBibliographicText(alias) {
				return true
			}
		}
		return false
	}
	if journal.ObservedTitle != nil {
		if err := requireText(*journal.ObservedTitle, "observed journal title"); err != nil {
			return err
		}
		if !isKnown(*journal.ObservedTitle) {
			return violation("observed journal title is not canonical or an accepted alias")
		}
	}
	for _, alias := range journal.ObservedTitleAliases {
		if err := requireText(alias, "observed title alias"); err != nil {
			return err
		}
		if !isKnown(alias) {
			return violation("observed title alias is not maintained by the catalog")
		}
	}
	hasMatch := false
	for _, issn := range journal.ObservedIssns {
		if err := requireIssn(issn, "observed ISSN"); err != nil {
			return err
		}
		hasMatch = hasMatch || slices.Contains(catalog.AllIssns, issn)
	}
	if len(journal.ObservedIssns) > 0 && len(catalog.AllIssns) > 0 && !hasMatch {
		return violation("observed journal ISSNs contradict the maintained catalog")
	}
	return nil
}

func validateArticle(catalog domain.JournalCatalogEntry, article domain.ArticleDraft) error {
	if article.CatalogId != catalog.CatalogId {
		return violation("article must echo the requested catalog_id")
	}
	if err := requireArticleTitle(article.Title, article.Doi, "article title"); err != nil {
		return err
	}
	if err := optionalYear(article.PublicationYear, "article publication year"); err != nil {
		return err
	}
	if err := optionalDate(article.Date); err != nil {
		return err
	}
	if !yearAgrees(article.PublicationYear, article.Date) {
		return violation("article publication year and date must agree")
	}
	if err := optionalDoi(article.Doi, "DOI"); err != nil {
		return err
	}
	for index, doi := range article.RetractionDois {
		if err := optionalDoi(&doi, "retraction DOI"); err != nil {
			return err
		}
		if index > 0 && article.RetractionDois[index-1] >= doi {
			return violation("retraction DOIs must be sorted and duplicate-free")
		}
	}
	if article.Pmid != nil && !sameOptional(domain.NormalizePmid(*article.Pmid), *article.Pmid) {
		return violation("PMID must use canonical digits")
	}
	for _, author := range article.Authors {
		if err := requireText(author.DisplayName, "author display name"); err != nil {
			return err
		}
	}
	for _, field := range []optionalField{{article.IssueTitle, "article issue title"}, {article.Volume, "article volume"}, {article.IssueNumber, "article issue number"}, {article.StartPage, "article start page"}, {article.EndPage, "article end page"}, {article.AbstractText, "article abstract"}} {
		if err := optionalText(field.value, field.name); err != nil {
			return err
		}
	}
	if article.Doi == nil && article.Pmid == nil && !((article.PublicationYear != nil || article.Date != nil) && (article.Volume != nil || article.IssueNumber != nil || article.StartPage != nil)) {
		return violation("article needs DOI, PMID, or a complete bibliographic identity basis")
	}
	return nil
}

func validateIssue(issue domain.IssueDraft) error {
	if err := optionalYear(issue.PublicationYear, "issue publication year"); err != nil {
		return err
	}
	if err := optionalDate(issue.Date); err != nil {
		return err
	}
	if !yearAgrees(issue.PublicationYear, issue.Date) {
		return violation("issue publication year and date must agree")
	}
	for _, field := range []optionalField{{issue.Volume, "issue volume"}, {issue.Number, "issue number"}, {issue.Title, "issue title"}} {
		if err := optionalText(field.value, field.name); err != nil {
			return err
		}
	}
	if !(issue.PublicationYear != nil && (issue.Volume != nil || issue.Number != nil)) && issue.Date == nil && issue.Title == nil {
		return violation("issue needs year plus volume/number, or a date/title fallback")
	}
	return nil
}

type optionalField struct {
	value *string
	name  string
}

func sameOptional(value *string, expected string) bool { return value != nil && *value == expected }
func optionalText(value *string, field string) error {
	if value == nil {
		return nil
	}
	return requireText(*value, field)
}
func requireText(value, field string) error {
	if !sameOptional(domain.NormalizeText(value), value) {
		return violation(field + " must be non-empty, trimmed, and Unicode-normalized")
	}
	return nil
}
func requireIssn(value, field string) error {
	if !sameOptional(domain.NormalizeIssn(value), value) {
		return violation(field + " must use canonical NNNN-NNNX form")
	}
	return nil
}
func optionalDoi(value *string, field string) error {
	if value != nil && !sameOptional(domain.NormalizeDoi(*value), *value) {
		return violation(field + " must use canonical DOI form")
	}
	return nil
}
func requireArticleTitle(title string, doi *string, field string) error {
	if title == "" && doi != nil && sameOptional(domain.NormalizeDoi(*doi), *doi) {
		return nil
	}
	return requireText(title, field)
}
func optionalYear(value *int64, field string) error {
	if value != nil && (*value < 1000 || *value > 9999) {
		return violation(field + " must use a four-digit positive year")
	}
	return nil
}
func yearAgrees(year *int64, date *string) bool {
	if year == nil || date == nil {
		return true
	}
	if len(*date) < 4 {
		return false
	}
	parsed, err := strconv.ParseInt((*date)[:4], 10, 64)
	return err == nil && parsed == *year
}
func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}

func validateCatalogId(value string) error {
	isValid := len(value) >= 3 && len(value) <= 128
	for index, character := range []byte(value) {
		if !(character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || index > 0 && (character == '.' || character == '_' || character == '-')) {
			isValid = false
			break
		}
	}
	if !isValid {
		return violation("catalog_id must match the canonical ASCII format")
	}
	return nil
}

func optionalDate(value *string) error {
	if value == nil {
		return nil
	}
	if !sameOptional(domain.NormalizeText(*value), *value) {
		return violation("date must use canonical text")
	}
	parts := strings.Split(*value, "-")
	isValid := len(parts) >= 1 && len(parts) <= 3 && len(parts[0]) == 4
	for index, part := range parts {
		if index > 0 && len(part) != 2 {
			isValid = false
		}
		for _, character := range []byte(part) {
			if character < '0' || character > '9' {
				isValid = false
			}
		}
	}
	if !isValid {
		return violation("date must use YYYY, YYYY-MM, or YYYY-MM-DD")
	}
	year, _ := strconv.ParseInt(parts[0], 10, 64)
	if err := optionalYear(&year, "date year"); err != nil {
		return err
	}
	if len(parts) >= 2 {
		month, _ := strconv.Atoi(parts[1])
		if month < 1 || month > 12 {
			return violation("date month is invalid")
		}
	}
	if len(parts) == 3 {
		day, _ := strconv.Atoi(parts[2])
		if day < 1 || day > 31 {
			return violation("date day is invalid")
		}
	}
	return nil
}

func failureKind(err error) ErrorKind {
	var providerError *Error
	if errors.As(err, &providerError) {
		return providerError.Kind
	}
	return Internal
}
