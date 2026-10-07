package cfp

import (
	"context"
	"slices"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ParsePage accepts only complete scoped publisher cards or explicitly verified empty statements.
func ParsePage(config SourceConfig, document Document, checkedOn string, isDiscovery bool) (ParsedPage, error) {
	parser, text, err := preparePageParser(config, document, checkedOn, isDiscovery)
	if err != nil {
		return ParsedPage{}, err
	}
	callBlocks, err := pageCallBlocks(config, text, isDiscovery)
	if err != nil {
		return ParsedPage{}, err
	}
	for _, block := range callBlocks {
		if err := parser.parseCard(block); err != nil {
			return ParsedPage{}, err
		}
	}
	return parser.complete(text)
}

// pageParser owns ordered card observations and rejects a complete page when any card is ambiguous.
type pageParser struct {
	config           SourceConfig
	document         Document
	checkedOn        string
	isDiscovery      bool
	parsed           *goquery.Document
	location         *whatwg.Url
	result           ParsedPage
	hasAmbiguousCard bool
}

// preparePageParser preserves body, URL, challenge and discovery preflight order.
func preparePageParser(config SourceConfig, document Document, checkedOn string, isDiscovery bool) (*pageParser, string, error) {
	if len(document.Text) > MaxPageBytes {
		return nil, "", ErrTooLarge
	}
	location, err := whatwg.NewParser().Parse(document.FinalUrl)
	if err != nil || !config.PermitsUrl(location) {
		return nil, "", ErrDisallowedUrl
	}
	parsed := parseHtml(document.Text)
	text := document.Text
	if document.Format == "html" {
		text = visible(parsed, "")
	}
	if IsChallenge(document) {
		return nil, "", ErrChallenge
	}
	if err := checkDiscoveryPage(config, parsed, text, isDiscovery); err != nil {
		return nil, "", err
	}
	return &pageParser{config: config, document: document, checkedOn: checkedOn, isDiscovery: isDiscovery, parsed: parsed, location: location, result: ParsedPage{Sources: []domain.Source{}, EmptyJournals: []domain.EmptyJournal{}, DetailUrls: []string{}, DetailTitles: map[string]string{}}}, text, nil
}

// checkDiscoveryPage admits scoped journal identity before rejecting pagination.
func checkDiscoveryPage(config SourceConfig, parsed *goquery.Document, text string, isDiscovery bool) error {
	if isDiscovery && !hasJournalIdentity(config, parsed, text) {
		return ErrUnrecognized
	}
	if isDiscovery && parsed.Find("a[rel~=next][href]").Length() > 0 {
		return ErrUnrecognized
	}
	return nil
}

// pageCallBlocks preserves adapter-specific heading scopes and the first detail heading.
func pageCallBlocks(config SourceConfig, text string, isDiscovery bool) ([]headingBlock, error) {
	var callBlocks []headingBlock
	switch {
	case config.Adapter == SnapshotOnly:
		return nil, ErrUnsupported
	case config.Adapter == SpringerCollections && isDiscovery:
		callBlocks = blocks(strings.SplitN(text, "\n## Journal navigation", 2)[0], 2, 3)
	case (config.Adapter == ElsevierCalls || config.Adapter == KeaiCalls) && isDiscovery:
		for _, block := range blocks(text, 1, 2) {
			if equalAscii(block.title, "Call for papers") {
				callBlocks = blocks(block.body, 3, 3)
				break
			}
		}
	default:
		callBlocks = blocks(text, 1, 1)
		if len(callBlocks) > 1 {
			callBlocks = callBlocks[:1]
		}
	}
	return callBlocks, nil
}

// addDetail assigns the latest title while retaining one URL for later sorted traversal.
func (parser *pageParser) addDetail(link *whatwg.Url, title string) {
	url := link.Href(false)
	parser.result.DetailTitles[url] = title
	if !slices.Contains(parser.result.DetailUrls, url) {
		parser.result.DetailUrls = append(parser.result.DetailUrls, url)
	}
}

// parseCard projects one scoped card and preserves incomplete-card routing before source publication.
func (parser *pageParser) parseCard(block headingBlock) error {
	title, body := block.title, block.body
	if parser.ignoresCard(title) || parser.skipsSpringerCard(block) {
		return nil
	}
	link := firstCardLink(parser.parsed, parser.document.FinalUrl, title)
	if parser.handlesBlankCard(title, body, link) {
		return nil
	}
	if !parser.isDiscovery && !pattern(`(?i)call for|submission|deadline|special issue|截[稿止]|征稿`, title+"\n"+body) {
		return ErrUnrecognized
	}
	status := capturedCardStatus(body)
	source, err := record(parser.config, parser.document, parser.checkedOn, title, body, status)
	if err != nil {
		return err
	}
	if !parser.isDiscovery && linksUndatedPdf(parser.parsed, source, body) {
		return ErrUnrecognized
	}
	if parser.handlesIncompleteCard(source, link, title) {
		return nil
	}
	parser.addSource(source, link)
	return nil
}

// ignoresCard excludes navigation and editorial headings before adapter admission.
func (parser *pageParser) ignoresCard(title string) bool {
	if pattern(`(?i)^(book reviews?|corrections?|editorials?|archive|meet the editors|latest articles|cookie|manage consent)`, title) || pattern(`(?i)^(collections|open collections|collections and calls for papers|calls? for papers|special issue calls? for papers|submission (?:instructions|guidelines|status|deadline)|guest editors?|filter by|journal navigation)$`, title) || title == parser.config.JournalTitle {
		return true
	}
	return false
}

// skipsSpringerCard rejects missing status and leaves nested collection cards to their own blocks.
func (parser *pageParser) skipsSpringerCard(block headingBlock) bool {
	body := block.body
	if parser.config.Adapter == SpringerCollections && parser.isDiscovery {
		if !pattern(statusPattern, body) {
			parser.hasAmbiguousCard = true
			return true
		}
		if slices.ContainsFunc(blocks(body, 2, 3), func(block headingBlock) bool { return pattern(statusPattern, block.body) }) {
			return true
		}
	}
	return false
}

// firstCardLink stops at the first exact title anchor, including an invalid first destination.
func firstCardLink(parsed *goquery.Document, base, title string) *whatwg.Url {
	var link *whatwg.Url
	for _, node := range parsed.Find("a[href]").Nodes {
		element := goquery.NewDocumentFromNode(node)
		if domain.CleanText(element.Text()) == title {
			href, _ := element.Attr("href")
			link, _ = whatwg.NewParser().ParseRef(base, href)
			break
		}
	}
	return link
}

// handlesBlankCard routes blank discovery cards to details before marking incomplete cards ambiguous.
func (parser *pageParser) handlesBlankCard(title, body string, link *whatwg.Url) bool {
	if strings.TrimSpace(body) == "" && parser.isDiscovery && parser.config.PermitsUrl(link) {
		parser.addDetail(link, title)
		return true
	}
	if strings.TrimSpace(title) == "" || strings.TrimSpace(body) == "" {
		parser.hasAmbiguousCard = true
		return true
	}
	return false
}

// linksUndatedPdf detects a downloadable original that cannot yet supply a complete detail record.
func linksUndatedPdf(parsed *goquery.Document, source domain.Source, body string) bool {
	if source.DateText == "" && source.RawDateText == "" && pattern(`(?i)(?:download|view).{0,40}(?:call for papers|pdf)`, body) {
		for _, node := range parsed.Find("a[href]").Nodes {
			href, _ := attribute(node, "href")
			if strings.HasSuffix(asciiLower(strings.SplitN(href, "?", 2)[0]), ".pdf") {
				return true
			}
		}
	}
	return false
}

// handlesIncompleteCard defers non-Springer discovery cards with no date or scope to permitted details.
func (parser *pageParser) handlesIncompleteCard(source domain.Source, link *whatwg.Url, title string) bool {
	if parser.isDiscovery && parser.config.Adapter != SpringerCollections && source.DateText == "" && source.RawDateText == "" && source.Scope == "" {
		if parser.config.PermitsUrl(link) {
			parser.addDetail(link, title)
			return true
		}
		parser.hasAmbiguousCard = true
		return true
	}
	return false
}

// complete rejects ambiguous or unverified empty pages before sorting linked details.
func (parser *pageParser) complete(text string) (ParsedPage, error) {
	if parser.hasAmbiguousCard {
		return ParsedPage{}, ErrUnrecognized
	}
	if len(parser.result.Sources) == 0 && len(parser.result.DetailUrls) == 0 {
		for _, statement := range parser.config.EmptyStatements {
			if strings.Contains(text, statement) {
				parser.result.EmptyJournals = append(parser.result.EmptyJournals, domain.EmptyJournal{CatalogIds: append([]string{}, parser.config.CatalogIds...), JournalTitle: parser.config.JournalTitle, CheckedOn: parser.checkedOn, SourceUrl: parser.document.FinalUrl, SourceStatement: statement, Notices: []domain.Notice{}})
				break
			}
		}
		if len(parser.result.EmptyJournals) == 0 {
			return ParsedPage{}, ErrUnrecognized
		}
	}
	slices.Sort(parser.result.DetailUrls)
	return parser.result, nil
}

// Acquire discovers notices and requires every linked detail before publishing a complete snapshot.
func Acquire(ctx context.Context, transport Transport, config SourceConfig, checkedOn string, deadline time.Time) (Acquisition, error) {
	if !config.CanRefresh() {
		return Acquisition{}, ErrUnsupported
	}
	document, err := transport.Fetch(ctx, config, config.DiscoveryUrl, deadline)
	if err != nil {
		return Acquisition{}, err
	}
	parsed, err := ParsePage(config, document, checkedOn, true)
	if err != nil {
		return Acquisition{}, err
	}
	result := Acquisition{Documents: []Document{document}, Sources: parsed.Sources, EmptyJournals: parsed.EmptyJournals}
	if len(parsed.DetailUrls) > MaxDetailPages {
		return Acquisition{}, ErrTooLarge
	}
	for _, url := range parsed.DetailUrls {
		if !time.Now().Before(deadline) {
			return Acquisition{}, ErrDeadline
		}
		if err := acquireLinkedDetail(ctx, transport, config, checkedOn, deadline, parsed.DetailTitles, url, &result); err != nil {
			return Acquisition{}, err
		}
	}
	if inconsistentAcquisition(result) {
		return Acquisition{}, ErrUnrecognized
	}
	return result, nil
}

// inconsistentAcquisition rejects missing outcomes and mixed notices with empty-journal declarations.
func inconsistentAcquisition(result Acquisition) bool {
	return len(result.Sources) == 0 && len(result.EmptyJournals) == 0 || len(result.Sources) > 0 && len(result.EmptyJournals) > 0
}

// prepareLinkedDocument verifies a PDF's expected title before supplying a synthetic detail heading.
func prepareLinkedDocument(document Document, titles map[string]string, url string) (Document, error) {
	parsingDocument := document
	if document.Format == "pdf_text" {
		title, has := titles[url]
		if !has || !strings.Contains(lower(strings.Join(strings.Fields(document.Text), " ")), lower(strings.Join(strings.Fields(title), " "))) {
			return Document{}, ErrUnrecognized
		}
		parsingDocument.Text = "# " + title + "\n" + document.Text
		parsingDocument.Format = "linked_pdf_text"
	}
	return parsingDocument, nil
}

// parseLinkedDetail requires at least one source and the expected title before acquisition can merge it.
func parseLinkedDetail(config SourceConfig, parsingDocument Document, checkedOn string, titles map[string]string, url string) ([]domain.Source, error) {
	detail, err := ParsePage(config, parsingDocument, checkedOn, false)
	if err != nil {
		return nil, err
	}
	if len(detail.Sources) == 0 {
		return nil, ErrUnrecognized
	}
	if title, has := titles[url]; has && !slices.ContainsFunc(detail.Sources, func(source domain.Source) bool { return equalAscii(source.Title, title) }) {
		return nil, ErrUnrecognized
	}
	return detail.Sources, nil
}

// mergeDetailSources replaces exact title matches in place and appends newly observed titles.
func mergeDetailSources(result *Acquisition, sources []domain.Source) {
	for _, source := range sources {
		index := slices.IndexFunc(result.Sources, func(existing domain.Source) bool { return existing.Title == source.Title })
		if index >= 0 {
			result.Sources[index] = source
		} else {
			result.Sources = append(result.Sources, source)
		}
	}
}

// capturedCardStatus selects the first status capture from the plain card body.
func capturedCardStatus(body string) string {
	status := ""
	plainBody := plain(body)
	if found := domain.PatternCaptures(statusPattern, plainBody); len(found) > 0 {
		status = plainBody[found[0][0]:found[0][1]]
	}
	return status
}

// addSource preserves the original URL unless a permitted title anchor has a different pathname.
func (parser *pageParser) addSource(source domain.Source, link *whatwg.Url) {
	if parser.config.PermitsUrl(link) && link.Pathname() != parser.location.Pathname() {
		source.SourceUrl = link.Href(false)
	}
	parser.result.Sources = append(parser.result.Sources, source)
}

// acquireLinkedDetail verifies and merges one detail before enforcing the aggregate capture limit.
func acquireLinkedDetail(ctx context.Context, transport Transport, config SourceConfig, checkedOn string, deadline time.Time, titles map[string]string, url string, result *Acquisition) error {
	document, err := transport.Fetch(ctx, config, url, deadline)
	if err != nil {
		return err
	}
	parsingDocument, err := prepareLinkedDocument(document, titles, url)
	if err != nil {
		return err
	}
	sources, err := parseLinkedDetail(config, parsingDocument, checkedOn, titles, url)
	if err != nil {
		return err
	}
	mergeDetailSources(result, sources)
	result.Documents = append(result.Documents, document)
	total := 0
	for _, document := range result.Documents {
		total += len(document.Text)
	}
	if total > MaxCaptureBytes {
		return ErrTooLarge
	}
	return nil
}
