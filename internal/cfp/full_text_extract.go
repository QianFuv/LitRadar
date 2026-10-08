package cfp

import (
	"regexp"
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/PuerkitoBio/goquery"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func firstMatch(pattern, text string) []int {
	found := domain.PatternCaptures(pattern, text)
	if len(found) == 0 {
		return nil
	}
	return found[0]
}
func cutBefore(pattern, text string) string {
	if found := firstMatch(pattern, text); found != nil {
		text = text[:found[0]]
	}
	return strings.TrimSpace(text)
}
func fullTextSections(body string) (string, string) {
	body = cutBefore(`(?im)^(?:[一二三四五六七八九十\d]+[、.．\s]+)?(?:references|bibliography|参考文献)\s*[:：]?\s*$`, plain(body))
	boundary := firstMatch(`(?im)^(?:(?:[一二三四五六七八九十\d]+|[IVX]+)[、.．\s]+)?(?:submissions?(?:\s*[:：]|[ \t]*$)|submissions? (?:format|guidelines?|instructions|information|requirements|procedure|process)|special issue submission and review process|all manuscripts will be reviewed as a cohort|all submissions must be formatted|all papers are to be submitted|instructions for authors|manuscript (?:preparation|requirements|submission)|author (?:guidelines|instructions)|how to submit|paper submission|稿件要求|投稿要求|征稿要求|投稿方式|投稿渠道|投稿网址|投稿指南|论文要求|提交要求|征文要求|来稿要求|征文投稿说明|收稿形式与评审流程|稿件提交|authors should prepare|prospective authors should submit|authors are encouraged to contact the editorial team|submitted papers should|papers must (?:be submitted|follow))`, body)
	if boundary == nil {
		return body, ""
	}
	return strings.TrimSpace(body[:boundary[0]]), strings.TrimSpace(body[boundary[0]:])
}
func findTitledContainer(parsed *goquery.Document, containerSelector, titleSelector, title string) *goquery.Selection {
	for _, node := range parsed.Find(containerSelector).Nodes {
		selection := goquery.NewDocumentFromNode(node).Selection
		if hasTitle(selection, titleSelector, title) {
			return selection
		}
	}
	return nil
}
func selectedBody(container *goquery.Selection, selector string) (string, error) {
	if container == nil {
		return "", ErrUnrecognized
	}
	body := container.Find(selector).First()
	if body.Length() == 0 {
		return "", ErrUnrecognized
	}
	return visible(fragment(inner(body)), ""), nil
}

// ExtractFullText replaces only body fields and URL; the original title and admission semantics survive.
func ExtractFullText(original domain.Source, document Document) (domain.Source, error) {
	if len(document.Text) > MaxPageBytes || IsChallenge(document) {
		return domain.Source{}, ErrUnrecognized
	}
	parsed := parseHtml(document.Text)
	collection := parsed.Find("[data-test='collection-title']").First()
	body, err := selectFullTextBody(original, document, parsed, collection)
	if err != nil {
		return domain.Source{}, err
	}
	scope, requirements := admittedFullTextSections(original, body)
	if incompleteFullText(original, scope, requirements) {
		return domain.Source{}, ErrUnrecognized
	}
	if collection.Length() == 0 && document.Format != "pdf_text" && hasDifferentOriginalLink(original, document, parsed) {
		return domain.Source{}, ErrUnrecognized
	}
	result := original
	result.Scope = scope
	result.Requirements = requirements
	result.SourceUrl = document.FinalUrl
	if domain.ParseSource(result) == nil {
		return domain.Source{}, ErrUnrecognized
	}
	return result, nil
}

func grslBody(parsed *goquery.Document, title string) (string, error) {
	matchesTitle := func(value string) bool {
		value = strings.TrimSpace(value)
		for _, prefix := range []string{"", "GRSS Special Stream on ", "GRSS Special Stream of the "} {
			if strings.HasPrefix(value, prefix) && matchingTitle(strings.TrimPrefix(value, prefix), title) {
				return true
			}
		}
		return false
	}
	var article *goquery.Selection
	for _, node := range parsed.Find("[data-elementor-type='single-post'].category-grsl-special-streams").Nodes {
		candidate := goquery.NewDocumentFromNode(node)
		for _, node := range candidate.Find(".elementor-widget-theme-post-title h1").Nodes {
			if matchesTitle(goquery.NewDocumentFromNode(node).Text()) {
				article = candidate.Selection
				break
			}
		}
		if article != nil {
			break
		}
	}
	body, err := selectedBody(article, ".elementor-widget-theme-post-content > .elementor-widget-container")
	if err != nil {
		return "", err
	}
	text := plain(body)
	if first, rest, has := strings.Cut(text, "\n"); has && matchesTitle(first) {
		text = rest
	}
	requirements := firstMatch(`(?im)^all submissions must be formatted`, text)
	if requirements == nil {
		return "", ErrUnrecognized
	}
	scope := cutBefore(`(?im)^(?:guest editors?|schedule)\s*[:：]?\s*$`, text[:requirements[0]])
	return scope + "\n" + strings.TrimSpace(text[requirements[0]:]), nil
}

func pdfBody(original domain.Source, document Document, isFamily func(string, string, string) bool) (string, error) {
	text := plain(document.Text)
	if pattern(`(?im)^(?:the exchange|table of contents|contents|newsletter|members in the news|aaea members in the news|news and announcements|job announcements|anti.harassment and code of conduct policy)\s*$`, text) {
		return "", ErrUnrecognized
	}
	matched := pdfTitleMatch(original.Title, text, isFamily)
	if matched == nil || utf8.RuneCountInString(text[:matched[0]]) > 600 {
		return "", ErrUnrecognized
	}
	body := strings.TrimSpace(text[matched[1]:])
	location, _ := whatwg.NewParser().Parse(document.FinalUrl)
	if isFamily("www.poms.org", "/sites/default/files/callforpapers/", "issn-1059-1478") && location.Pathname() == "/sites/default/files/callforpapers/FlexMfgEcosystems-Revised_0.pdf" {
		return pomsManufacturingBody(body)
	}
	return body, nil
}

// selectFullTextBody retains ordered publisher-family precedence before PDF and generic extraction.
func selectFullTextBody(original domain.Source, document Document, parsed *goquery.Document, collection *goquery.Selection) (string, error) {
	location, _ := whatwg.NewParser().Parse(document.FinalUrl)
	isFamily := func(host, path, id string) bool {
		return location != nil && location.Hostname() == host && strings.HasPrefix(location.Pathname(), path) && slices.Contains(original.CatalogIds, id)
	}
	if collection.Length() > 0 {
		return collectionBody(parsed, collection, original.Title)
	}
	if body := springerUpdateBody(original, document, parsed); body != nil {
		return springerFullTextBody(body)
	}
	switch {
	case isFamily("www.comsoc.org", "/publications/journals/ieee-jsac/cfp/", "issn-0733-8716") || isFamily("www.comsoc.org", "/publications/journals/ieee-tnsm/cfp/", "issn-1932-4537"):
		return comsocBody(parsed, original.Title)
	case isFamily("journal.psych.ac.cn", "/xlxb/CN/news/", "issn-0439-755x"):
		return selectedBody(findTitledContainer(parsed, ".content_nr", ".item_biaoti", original.Title), ".J_WenZhang")
	case isFamily("www.resci.cn", "/CN/news/", "issn-1007-7588"):
		return resciBody(parsed, original.Title)
	case isFamily("chinaifs.org.cn", "/html/web/tongzhigonggao/", "issn-1006-1029"):
		return chinaifsBody(parsed, original.Title)
	case isFamily("www.jryj.org.cn", "/CN/news/", "issn-1002-7246"):
		return jryjBody(parsed, original.Title)
	case isFamily("kxxyj.magtechjournal.com", "/kxxyj/CN/news/", "issn-1003-2053"):
		return magtechBody(parsed, original.Title)
	case isFamily("www.poms.org", "/node/", "issn-1059-1478"):
		return selectedBody(findTitledContainer(parsed, "article.node--type-call-for-papers.node--view-mode-full", "h1.node__title", original.Title), ".field-name-field-submission-guidelines-summ")
	case isFamily("www.grss-ieee.org", "/publications/author-resources/grsl-special-streams/", "issn-1545-598x"):
		return grslBody(parsed, original.Title)
	case document.Format == "pdf_text":
		return pdfBody(original, document, isFamily)
	default:
		return genericFullTextBody(parsed, original.Title)
	}
}

// collectionBody selects the original publisher body after its title admission checks.
func collectionBody(parsed *goquery.Document, collection *goquery.Selection, title string) (string, error) {
	body := ""
	if !matchingTitle(collection.Text(), title) {
		return "", ErrUnrecognized
	}
	description := parsed.Find("[data-test='collection-description']").First()
	if description.Length() == 0 {
		return "", ErrUnrecognized
	}
	body = visible(fragment(inner(description)), "")
	return body, nil
}

// springerFullTextBody cleans the selected publisher body after its title admission checks.
func springerFullTextBody(selection *goquery.Selection) (string, error) {
	body := visible(fragment(inner(selection)), "h1, .u-visually-hidden")
	if pattern(`(?i)read the full call for papers`, body) {
		return "", ErrUnrecognized
	}
	body = cutBefore(`(?im)^(?:#{1,6} )?bios of guest editors\s*[:：]?\s*$`, body)
	return body, nil
}

// comsocBody selects the original publisher body after its title admission checks.
func comsocBody(parsed *goquery.Document, title string) (string, error) {
	body := ""
	if !hasTitle(parsed.Selection, "h1.h1--page-title", title) {
		return "", ErrUnrecognized
	}
	article := parsed.Find("article.node--type-call-for-papers.node--view-mode-full").First()
	if article.Length() == 0 {
		return "", ErrUnrecognized
	}
	parts := []string{}
	for _, node := range article.Find(".paragraph-anchor-wrapper .text-long").Nodes {
		parts = append(parts, visible(fragment(inner(goquery.NewDocumentFromNode(node).Selection)), ""))
	}
	text := plain(strings.Join(parts, "\n"))
	scopeHeading := firstMatch(`(?im)^(?:scope|call for papers)\s*[:：]?\s*$`, text)
	scopeStart := 0
	if scopeHeading != nil {
		scopeStart = scopeHeading[1]
	} else if strings.HasPrefix(lower(text), "important dates") {
		return "", ErrUnrecognized
	}
	requirement := firstMatch(`(?im)^submissions? (?:format|guidelines)\s*[:：]?\s*$`, text)
	if requirement == nil || scopeStart >= requirement[0] {
		return "", ErrUnrecognized
	}
	end := `(?im)^(?:important dates|guest editors|references)\s*[:：]?\s*$`
	body = cutBefore(end, text[scopeStart:requirement[0]]) + "\n" + cutBefore(end, text[requirement[0]:])
	return body, nil
}

// resciBody selects the original publisher body after its title admission checks.
func resciBody(parsed *goquery.Document, title string) (string, error) {
	body := ""
	container := findTitledContainer(parsed, ".content_nr > .news-content", ".newstitle", title)
	if container == nil {
		return "", ErrUnrecognized
	}
	body = visible(fragment(inner(container)), ".newstitle, .text-right")
	return body, nil
}

// chinaifsBody selects the original publisher body after its title admission checks.
func chinaifsBody(parsed *goquery.Document, title string) (string, error) {
	var container *goquery.Selection
	for _, node := range parsed.Find(".news-wrap").Nodes {
		candidate := goquery.NewDocumentFromNode(node)
		titles := []string{}
		for _, node := range candidate.Find(".news-title > h1, .news-title > h2").Nodes {
			titles = append(titles, goquery.NewDocumentFromNode(node).Text())
		}
		if matchingTitle(strings.Join(titles, "\n"), title) {
			container = candidate.Selection
			break
		}
	}
	return selectedBody(container, ".news-content")
}

// jryjBody selects the original publisher body after its title admission checks.
func jryjBody(parsed *goquery.Document, title string) (string, error) {
	var table *goquery.Selection
	for _, node := range parsed.Find("td.news_biaoti").Nodes {
		heading := goquery.NewDocumentFromNode(node)
		if matchingTitle(heading.Text(), title) {
			table = heading.ParentsFiltered("table").First()
			break
		}
	}
	return selectedBody(table, "span.J_WenZhang")
}

// magtechBody selects the original publisher body after its title admission checks.
func magtechBody(parsed *goquery.Document, title string) (string, error) {
	body := ""
	container := findTitledContainer(parsed, ".content_nr > .item_con > ul", ".item_biaoti", title)
	if container == nil {
		return "", ErrUnrecognized
	}
	body = visible(fragment(inner(container)), ".item_biaoti")
	return body, nil
}

// genericFullTextBody selects the original publisher body after its title admission checks.
func genericFullTextBody(parsed *goquery.Document, title string) (string, error) {
	body := ""
	if !hasTitle(parsed.Selection, "h1,h2", title) {
		return "", ErrUnrecognized
	}
	article := parsed.Find("article.general-post-content .prose, [itemprop='articleBody'], .entry-content, .article-content, .c-article-body").First()
	if article.Length() == 0 {
		return "", ErrUnrecognized
	}
	body = visible(fragment(inner(article)), "")
	body = cutBefore(`(?im)^(?:#{1,6} )?(?:journal navigation|related articles|related content|latest articles|latest news|read next|about springer nature link|footer navigation|navigation|search|references|cookie preferences)\s*$`, body)
	return body, nil
}

// admittedFullTextSections keeps resource-science notes with submission requirements.
func admittedFullTextSections(original domain.Source, body string) (string, string) {
	scope, requirements := fullTextSections(body)
	if slices.Contains(original.CatalogIds, "issn-1007-7588") {
		if notes := firstMatch(`(?m)^(?:重点注意事项|注意事项|时间节点)\s*[:：]?\s*$`, scope); notes != nil {
			requirements = strings.TrimSpace(strings.TrimSpace(scope[notes[0]:]) + "\n" + requirements)
			scope = strings.TrimSpace(scope[:notes[0]])
		}
	}
	return scope, requirements
}

// incompleteFullText rejects missing required sections and truncated body text.
func incompleteFullText(original domain.Source, scope, requirements string) bool {
	return scope == "" && requirements == "" || original.Requirements != "" && requirements == "" || pattern(`(?m)(?:\.{3}|…)\s*$`, scope) || pattern(`(?m)(?:\.{3}|…)\s*$`, requirements)
}

// hasDifferentOriginalLink prevents an intermediate page from replacing its linked original.
func hasDifferentOriginalLink(original domain.Source, document Document, parsed *goquery.Document) bool {
	for _, node := range parsed.Find("a[href]").Nodes {
		link := goquery.NewDocumentFromNode(node)
		if matchingTitle(link.Text(), original.Title) {
			href, _ := link.Attr("href")
			joined, err := whatwg.NewParser().ParseRef(document.FinalUrl, href)
			if err == nil && strings.SplitN(joined.Href(false), "#", 2)[0] != strings.SplitN(document.FinalUrl, "#", 2)[0] {
				return true
			}
		}
	}
	return false
}

// pdfTitleMatch first matches quoted title words, then allows publisher-specific punctuation splitting.
func pdfTitleMatch(title, text string, isFamily func(string, string, string) bool) []int {
	words := strings.Fields(title)
	for index, word := range words {
		words[index] = regexp.QuoteMeta(word)
	}
	matched := firstMatch("(?i)"+strings.Join(words, `\s+`), text)
	if matched == nil && (isFamily("ieee-iotj.org", "/wp-content/uploads/", "issn-2327-4662") || isFamily("www.poms.org", "/sites/default/files/callforpapers/", "issn-1059-1478")) {
		characters := []string{}
		for _, character := range strings.ReplaceAll(title, "&", "and") {
			if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
				characters = append(characters, regexp.QuoteMeta(string(character)))
			}
		}
		matched = firstMatch("(?i)"+strings.Join(characters, `[\s\p{P}]*`), text)
	}
	return matched
}

// pomsManufacturingBody requires the original ordered scope, date, requirements and editor headings.
func pomsManufacturingBody(body string) (string, error) {
	scope := firstMatch(`(?im)^Background:`, body)
	dates := firstMatch(`(?im)^Deadlines\s*$`, body)
	requirements := firstMatch(`(?im)^Authors are encouraged to contact the editorial team`, body)
	editors := firstMatch(`(?im)^Guest Editors\s*$`, body)
	if scope == nil || dates == nil || requirements == nil || editors == nil || !(scope[0] < dates[0] && dates[1] <= requirements[0] && requirements[0] < editors[0]) {
		return "", ErrUnrecognized
	}
	return strings.TrimSpace(body[scope[0]:dates[0]]) + "\n" + strings.TrimSpace(body[requirements[0]:editors[0]]), nil
}
