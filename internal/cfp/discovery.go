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
	if len(document.Text) > MaxPageBytes {
		return ParsedPage{}, ErrTooLarge
	}
	location, err := whatwg.NewParser().Parse(document.FinalUrl)
	if err != nil || !config.PermitsUrl(location) {
		return ParsedPage{}, ErrDisallowedUrl
	}
	parsed := parseHtml(document.Text)
	text := document.Text
	if document.Format == "html" {
		text = visible(parsed, "")
	}
	if IsChallenge(document) {
		return ParsedPage{}, ErrChallenge
	}
	if isDiscovery && !hasJournalIdentity(config, parsed, text) {
		return ParsedPage{}, ErrUnrecognized
	}
	if isDiscovery && parsed.Find("a[rel~=next][href]").Length() > 0 {
		return ParsedPage{}, ErrUnrecognized
	}
	result := ParsedPage{Sources: []domain.Source{}, EmptyJournals: []domain.EmptyJournal{}, DetailUrls: []string{}, DetailTitles: map[string]string{}}
	var callBlocks []headingBlock
	switch {
	case config.Adapter == SnapshotOnly:
		return ParsedPage{}, ErrUnsupported
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
	hasAmbiguousCard := false
	addDetail := func(link *whatwg.Url, title string) {
		url := link.Href(false)
		result.DetailTitles[url] = title
		if !slices.Contains(result.DetailUrls, url) {
			result.DetailUrls = append(result.DetailUrls, url)
		}
	}
	for _, block := range callBlocks {
		title, body := block.title, block.body
		if pattern(`(?i)^(book reviews?|corrections?|editorials?|archive|meet the editors|latest articles|cookie|manage consent)`, title) || pattern(`(?i)^(collections|open collections|collections and calls for papers|calls? for papers|special issue calls? for papers|submission (?:instructions|guidelines|status|deadline)|guest editors?|filter by|journal navigation)$`, title) || title == config.JournalTitle {
			continue
		}
		if config.Adapter == SpringerCollections && isDiscovery {
			if !pattern(statusPattern, body) {
				hasAmbiguousCard = true
				continue
			}
			if slices.ContainsFunc(blocks(body, 2, 3), func(block headingBlock) bool { return pattern(statusPattern, block.body) }) {
				continue
			}
		}
		var link *whatwg.Url
		for _, node := range parsed.Find("a[href]").Nodes {
			element := goquery.NewDocumentFromNode(node)
			if domain.CleanText(element.Text()) == title {
				href, _ := element.Attr("href")
				link, _ = whatwg.NewParser().ParseRef(document.FinalUrl, href)
				break
			}
		}
		if strings.TrimSpace(body) == "" && isDiscovery && config.PermitsUrl(link) {
			addDetail(link, title)
			continue
		}
		if strings.TrimSpace(title) == "" || strings.TrimSpace(body) == "" {
			hasAmbiguousCard = true
			continue
		}
		if !isDiscovery && !pattern(`(?i)call for|submission|deadline|special issue|截[稿止]|征稿`, title+"\n"+body) {
			return ParsedPage{}, ErrUnrecognized
		}
		status := ""
		plainBody := plain(body)
		if found := domain.PatternCaptures(statusPattern, plainBody); len(found) > 0 {
			status = plainBody[found[0][0]:found[0][1]]
		}
		source, err := record(config, document, checkedOn, title, body, status)
		if err != nil {
			return ParsedPage{}, err
		}
		if !isDiscovery && source.DateText == "" && source.RawDateText == "" && pattern(`(?i)(?:download|view).{0,40}(?:call for papers|pdf)`, body) {
			for _, node := range parsed.Find("a[href]").Nodes {
				href, _ := attribute(node, "href")
				if strings.HasSuffix(asciiLower(strings.SplitN(href, "?", 2)[0]), ".pdf") {
					return ParsedPage{}, ErrUnrecognized
				}
			}
		}
		if isDiscovery && config.Adapter != SpringerCollections && source.DateText == "" && source.RawDateText == "" && source.Scope == "" {
			if config.PermitsUrl(link) {
				addDetail(link, title)
				continue
			}
			hasAmbiguousCard = true
			continue
		}
		if config.PermitsUrl(link) && link.Pathname() != location.Pathname() {
			source.SourceUrl = link.Href(false)
		}
		result.Sources = append(result.Sources, source)
	}
	if hasAmbiguousCard {
		return ParsedPage{}, ErrUnrecognized
	}
	if len(result.Sources) == 0 && len(result.DetailUrls) == 0 {
		for _, statement := range config.EmptyStatements {
			if strings.Contains(text, statement) {
				result.EmptyJournals = append(result.EmptyJournals, domain.EmptyJournal{CatalogIds: append([]string{}, config.CatalogIds...), JournalTitle: config.JournalTitle, CheckedOn: checkedOn, SourceUrl: document.FinalUrl, SourceStatement: statement, Notices: []domain.Notice{}})
				break
			}
		}
		if len(result.EmptyJournals) == 0 {
			return ParsedPage{}, ErrUnrecognized
		}
	}
	slices.Sort(result.DetailUrls)
	return result, nil
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
		document, err := transport.Fetch(ctx, config, url, deadline)
		if err != nil {
			return Acquisition{}, err
		}
		parsingDocument := document
		if document.Format == "pdf_text" {
			title, has := parsed.DetailTitles[url]
			if !has || !strings.Contains(lower(strings.Join(strings.Fields(document.Text), " ")), lower(strings.Join(strings.Fields(title), " "))) {
				return Acquisition{}, ErrUnrecognized
			}
			parsingDocument.Text = "# " + title + "\n" + document.Text
			parsingDocument.Format = "linked_pdf_text"
		}
		detail, err := ParsePage(config, parsingDocument, checkedOn, false)
		if err != nil {
			return Acquisition{}, err
		}
		if len(detail.Sources) == 0 {
			return Acquisition{}, ErrUnrecognized
		}
		if title, has := parsed.DetailTitles[url]; has && !slices.ContainsFunc(detail.Sources, func(source domain.Source) bool { return equalAscii(source.Title, title) }) {
			return Acquisition{}, ErrUnrecognized
		}
		for _, source := range detail.Sources {
			index := slices.IndexFunc(result.Sources, func(existing domain.Source) bool { return existing.Title == source.Title })
			if index >= 0 {
				result.Sources[index] = source
			} else {
				result.Sources = append(result.Sources, source)
			}
		}
		result.Documents = append(result.Documents, document)
		total := 0
		for _, document := range result.Documents {
			total += len(document.Text)
		}
		if total > MaxCaptureBytes {
			return Acquisition{}, ErrTooLarge
		}
	}
	if len(result.Sources) == 0 && len(result.EmptyJournals) == 0 || len(result.Sources) > 0 && len(result.EmptyJournals) > 0 {
		return Acquisition{}, ErrUnrecognized
	}
	return result, nil
}
