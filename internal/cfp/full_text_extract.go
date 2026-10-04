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
	location, _ := whatwg.NewParser().Parse(document.FinalUrl)
	isFamily := func(host, path, id string) bool {
		return location != nil && location.Hostname() == host && strings.HasPrefix(location.Pathname(), path) && slices.Contains(original.CatalogIds, id)
	}
	body := ""
	var err error
	switch {
	case collection.Length() > 0:
		if !matchingTitle(collection.Text(), original.Title) {
			return domain.Source{}, ErrUnrecognized
		}
		description := parsed.Find("[data-test='collection-description']").First()
		if description.Length() == 0 {
			return domain.Source{}, ErrUnrecognized
		}
		body = visible(fragment(inner(description)), "")
	case springerUpdateBody(original, document, parsed) != nil:
		body = visible(fragment(inner(springerUpdateBody(original, document, parsed))), "h1, .u-visually-hidden")
		if pattern(`(?i)read the full call for papers`, body) {
			return domain.Source{}, ErrUnrecognized
		}
		body = cutBefore(`(?im)^(?:#{1,6} )?bios of guest editors\s*[:：]?\s*$`, body)
	case isFamily("www.comsoc.org", "/publications/journals/ieee-jsac/cfp/", "issn-0733-8716") || isFamily("www.comsoc.org", "/publications/journals/ieee-tnsm/cfp/", "issn-1932-4537"):
		if !hasTitle(parsed.Selection, "h1.h1--page-title", original.Title) {
			return domain.Source{}, ErrUnrecognized
		}
		article := parsed.Find("article.node--type-call-for-papers.node--view-mode-full").First()
		if article.Length() == 0 {
			return domain.Source{}, ErrUnrecognized
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
			return domain.Source{}, ErrUnrecognized
		}
		requirement := firstMatch(`(?im)^submissions? (?:format|guidelines)\s*[:：]?\s*$`, text)
		if requirement == nil || scopeStart >= requirement[0] {
			return domain.Source{}, ErrUnrecognized
		}
		end := `(?im)^(?:important dates|guest editors|references)\s*[:：]?\s*$`
		body = cutBefore(end, text[scopeStart:requirement[0]]) + "\n" + cutBefore(end, text[requirement[0]:])
	case isFamily("journal.psych.ac.cn", "/xlxb/CN/news/", "issn-0439-755x"):
		body, err = selectedBody(findTitledContainer(parsed, ".content_nr", ".item_biaoti", original.Title), ".J_WenZhang")
	case isFamily("www.resci.cn", "/CN/news/", "issn-1007-7588"):
		container := findTitledContainer(parsed, ".content_nr > .news-content", ".newstitle", original.Title)
		if container == nil {
			return domain.Source{}, ErrUnrecognized
		}
		body = visible(fragment(inner(container)), ".newstitle, .text-right")
	case isFamily("chinaifs.org.cn", "/html/web/tongzhigonggao/", "issn-1006-1029"):
		var container *goquery.Selection
		for _, node := range parsed.Find(".news-wrap").Nodes {
			candidate := goquery.NewDocumentFromNode(node)
			titles := []string{}
			for _, node := range candidate.Find(".news-title > h1, .news-title > h2").Nodes {
				titles = append(titles, goquery.NewDocumentFromNode(node).Text())
			}
			if matchingTitle(strings.Join(titles, "\n"), original.Title) {
				container = candidate.Selection
				break
			}
		}
		body, err = selectedBody(container, ".news-content")
	case isFamily("www.jryj.org.cn", "/CN/news/", "issn-1002-7246"):
		var table *goquery.Selection
		for _, node := range parsed.Find("td.news_biaoti").Nodes {
			title := goquery.NewDocumentFromNode(node)
			if matchingTitle(title.Text(), original.Title) {
				table = title.ParentsFiltered("table").First()
				break
			}
		}
		body, err = selectedBody(table, "span.J_WenZhang")
	case isFamily("kxxyj.magtechjournal.com", "/kxxyj/CN/news/", "issn-1003-2053"):
		container := findTitledContainer(parsed, ".content_nr > .item_con > ul", ".item_biaoti", original.Title)
		if container == nil {
			return domain.Source{}, ErrUnrecognized
		}
		body = visible(fragment(inner(container)), ".item_biaoti")
	case isFamily("www.poms.org", "/node/", "issn-1059-1478"):
		body, err = selectedBody(findTitledContainer(parsed, "article.node--type-call-for-papers.node--view-mode-full", "h1.node__title", original.Title), ".field-name-field-submission-guidelines-summ")
	case isFamily("www.grss-ieee.org", "/publications/author-resources/grsl-special-streams/", "issn-1545-598x"):
		body, err = grslBody(parsed, original.Title)
	case document.Format == "pdf_text":
		body, err = pdfBody(original, document, isFamily)
	default:
		if !hasTitle(parsed.Selection, "h1,h2", original.Title) {
			return domain.Source{}, ErrUnrecognized
		}
		article := parsed.Find("article.general-post-content .prose, [itemprop='articleBody'], .entry-content, .article-content, .c-article-body").First()
		if article.Length() == 0 {
			return domain.Source{}, ErrUnrecognized
		}
		body = visible(fragment(inner(article)), "")
		body = cutBefore(`(?im)^(?:#{1,6} )?(?:journal navigation|related articles|related content|latest articles|latest news|read next|about springer nature link|footer navigation|navigation|search|references|cookie preferences)\s*$`, body)
	}
	if err != nil {
		return domain.Source{}, err
	}
	scope, requirements := fullTextSections(body)
	if slices.Contains(original.CatalogIds, "issn-1007-7588") {
		if notes := firstMatch(`(?m)^(?:重点注意事项|注意事项|时间节点)\s*[:：]?\s*$`, scope); notes != nil {
			requirements = strings.TrimSpace(strings.TrimSpace(scope[notes[0]:]) + "\n" + requirements)
			scope = strings.TrimSpace(scope[:notes[0]])
		}
	}
	if scope == "" && requirements == "" || original.Requirements != "" && requirements == "" || pattern(`(?m)(?:\.{3}|…)\s*$`, scope) || pattern(`(?m)(?:\.{3}|…)\s*$`, requirements) {
		return domain.Source{}, ErrUnrecognized
	}
	if collection.Length() == 0 && document.Format != "pdf_text" {
		for _, node := range parsed.Find("a[href]").Nodes {
			link := goquery.NewDocumentFromNode(node)
			if matchingTitle(link.Text(), original.Title) {
				href, _ := link.Attr("href")
				joined, err := whatwg.NewParser().ParseRef(document.FinalUrl, href)
				if err == nil && strings.SplitN(joined.Href(false), "#", 2)[0] != strings.SplitN(document.FinalUrl, "#", 2)[0] {
					return domain.Source{}, ErrUnrecognized
				}
			}
		}
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
	words := strings.Fields(original.Title)
	for index, word := range words {
		words[index] = regexp.QuoteMeta(word)
	}
	matched := firstMatch("(?i)"+strings.Join(words, `\s+`), text)
	if matched == nil && (isFamily("ieee-iotj.org", "/wp-content/uploads/", "issn-2327-4662") || isFamily("www.poms.org", "/sites/default/files/callforpapers/", "issn-1059-1478")) {
		characters := []string{}
		for _, character := range strings.ReplaceAll(original.Title, "&", "and") {
			if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
				characters = append(characters, regexp.QuoteMeta(string(character)))
			}
		}
		matched = firstMatch("(?i)"+strings.Join(characters, `[\s\p{P}]*`), text)
	}
	if matched == nil || utf8.RuneCountInString(text[:matched[0]]) > 600 {
		return "", ErrUnrecognized
	}
	body := strings.TrimSpace(text[matched[1]:])
	location, _ := whatwg.NewParser().Parse(document.FinalUrl)
	if isFamily("www.poms.org", "/sites/default/files/callforpapers/", "issn-1059-1478") && location.Pathname() == "/sites/default/files/callforpapers/FlexMfgEcosystems-Revised_0.pdf" {
		scope := firstMatch(`(?im)^Background:`, body)
		dates := firstMatch(`(?im)^Deadlines\s*$`, body)
		requirements := firstMatch(`(?im)^Authors are encouraged to contact the editorial team`, body)
		editors := firstMatch(`(?im)^Guest Editors\s*$`, body)
		if scope == nil || dates == nil || requirements == nil || editors == nil || !(scope[0] < dates[0] && dates[1] <= requirements[0] && requirements[0] < editors[0]) {
			return "", ErrUnrecognized
		}
		return strings.TrimSpace(body[scope[0]:dates[0]]) + "\n" + strings.TrimSpace(body[requirements[0]:editors[0]]), nil
	}
	return body, nil
}
