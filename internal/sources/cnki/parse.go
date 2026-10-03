package cnki

import (
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// CheckedText classifies empty and verification responses before endpoint parsing.
func CheckedText(text string) error {
	if strings.TrimSpace(text) == "" {
		return &Error{Kind: "Parse", Message: "domestic CNKI returned an empty response"}
	}
	if (strings.Contains(domain.Lowercase(text), "captcha") || strings.Contains(text, "访问异常") || strings.Contains(text, "安全验证") || strings.Contains(text, `"code":-403`) || strings.Contains(text, "/verify/home")) && !looksLikeContent(text) {
		return &Error{Kind: "Request", Message: "domestic CNKI verification required"}
	}
	return nil
}
func containsMarker(text string, markers ...string) bool {
	for _, marker := range markers {
		if strings.Contains(text, marker) {
			return true
		}
	}
	return false
}
func explicitEmpty(text string) bool {
	return containsMarker(domain.Lowercase(text), "暂无数据", "暂无相关", "未检索到", "没有找到", "无相关记录", "共 0 条结果", "共0条结果", "找到 0 条结果", "找到0条结果", "no results")
}
func looksLikeContent(text string) bool {
	return containsMarker(text, "YearIssueTree", `id="pykm"`, "ChDivSummary", "/knavi/detail?", `class="row clearfix`, "param-filename", "paramfilename") || explicitEmpty(text)
}
func updatePlaceholder(text string) bool {
	return strings.Contains(stripTags(decodeHtml(text)), "该刊数据正在更新中，请耐心等待")
}
func validateResponse(endpoint, text string) error {
	if err := CheckedText(text); err != nil {
		return err
	}
	if endpoint == "article_detail" && containsMarker(domain.Lowercase(text), "记录已删除", "文献不存在", "该文献不存在", "record has been deleted", "record does not exist") {
		return &Error{Kind: "PermanentArticleMissing"}
	}
	hasStructure := false
	switch endpoint {
	case "navigation":
		lowered := asciiLower(text)
		hasStructure = strings.Contains(lowered, "<html") && strings.Contains(lowered, "</html>")
	case "journal_search":
		hasStructure = strings.Contains(text, "/knavi/detail?") || explicitEmpty(text)
	case "journal_detail":
		hasStructure = inputValue(text, "pykm") != nil || explicitEmpty(text)
	case "year_issues":
		hasStructure = strings.Contains(text, "YearIssueTree")
	case "issue_articles":
		hasStructure = strings.Contains(text, "articleCount") || strings.Contains(text, `class="row clearfix`) || updatePlaceholder(text)
	case "article_detail":
		hasStructure = inputValue(text, "paramfilename") != nil || inputValue(text, "param-filename") != nil
	}
	if !hasStructure {
		return &Error{Kind: "Parse", Message: fmt.Sprintf("domestic CNKI %s response is structurally incomplete", endpoint)}
	}
	return nil
}
func valueOrEmpty(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}
func optional(value *string) any {
	if value == nil {
		return nil
	}
	return *value
}
func firstValue(values ...*string) *string {
	for _, value := range values {
		if value != nil {
			return value
		}
	}
	return nil
}
func field(value any, key string) any { object, _ := value.(map[string]any); return object[key] }
func cloneJson(value any) any {
	switch typed := value.(type) {
	case map[string]any:
		copy := make(map[string]any, len(typed))
		for key, item := range typed {
			copy[key] = cloneJson(item)
		}
		return copy
	case []any:
		copy := make([]any, len(typed))
		for index, item := range typed {
			copy[index] = cloneJson(item)
		}
		return copy
	default:
		return value
	}
}

// ParseJournalSearch returns deduplicated domestic candidate URLs in upstream order.
func ParseJournalSearch(text string) ([]any, error) {
	if err := validateResponse("journal_search", text); err != nil {
		return nil, err
	}
	candidates := []any{}
	seen := map[string]bool{}
	for _, tag := range tags(text, "a") {
		values := attrs(tag)
		href, exists := values["href"]
		if !exists || !strings.Contains(href, "/knavi/detail?") {
			continue
		}
		url, err := WithDomesticPlatform(href)
		if err != nil {
			return nil, err
		}
		if ContainsOverseasHost(url) {
			return nil, &Error{Kind: "Parse", Message: "domestic journal search returned overseas host"}
		}
		if seen[url] {
			continue
		}
		seen[url] = true
		title := firstValue(nonEmpty(values["title"]), nonEmpty(stripTags(tag)))
		candidates = append(candidates, map[string]any{"title": valueOrEmpty(title), "issn": valueOrEmpty(nearbyIssn(text, href)), "detail_url": url})
	}
	return candidates, nil
}
func nearbyIssn(text, href string) *string {
	encoded := strings.ReplaceAll(href, "&", "&amp;")
	index := strings.Index(text, href)
	length := len(href)
	if index < 0 {
		index = strings.Index(text, encoded)
		length = len(encoded)
	}
	if index < 0 {
		return nil
	}
	start, end := max(index-80, 0), min(index+length+400, len(text))
	for start > 0 && !utf8.RuneStart(text[start]) {
		start--
	}
	for end < len(text) && !utf8.RuneStart(text[end]) {
		end++
	}
	return labelValue(stripTags(text[start:end]), "ISSN")
}

// ParseJournalDetail requires a product key and retains optional metadata as null.
func ParseJournalDetail(text string) (any, error) {
	if err := validateResponse("journal_detail", text); err != nil {
		return nil, err
	}
	pykm := inputValue(text, "pykm")
	if pykm == nil {
		return nil, &Error{Kind: "Parse", Message: "domestic journal detail missing pykm"}
	}
	pcode := "CJFD,CCJD"
	if value := inputValue(text, "pCode"); value != nil {
		pcode = *value
	}
	visible := stripTags(text)
	url, err := WithDomesticPlatform(NaviBase + "/knavi/detail?pykm=" + *pykm)
	if err != nil {
		return nil, err
	}
	if ContainsOverseasHost(url) {
		return nil, &Error{Kind: "Parse", Message: "domestic journal detail produced overseas host"}
	}
	return map[string]any{"detail_url": url, "pykm": *pykm, "pcode": pcode, "time": optional(inputValue(text, "time")), "title": optional(firstValue(inputValue(text, "shareChName"), titleText(text))), "issn": optional(labelValue(visible, "ISSN")), "cn": optional(labelValue(visible, "CN")), "raw_text": visible, "platform": "NZKPT"}, nil
}

// ParseYearIssues retains the upstream order and opaque issue token.
func ParseYearIssues(text string) ([]any, error) {
	if err := validateResponse("year_issues", text); err != nil {
		return nil, err
	}
	issues := []any{}
	for _, tag := range tags(text, "a") {
		values := attrs(tag)
		id := values["id"]
		if !strings.HasPrefix(id, "yq") {
			continue
		}
		key := id[2:]
		if len(key) < 4 || !utf8.ValidString(key[:4]) {
			continue
		}
		year, err := strconv.ParseInt(key[:4], 10, 64)
		if err != nil {
			continue
		}
		token, exists := values["value"]
		if !exists {
			continue
		}
		label := stripTags(tag)
		number := issueNumber(key, label)
		issues = append(issues, map[string]any{"year": year, "number": number, "title": label, "year_issue": decodeHtml(token), "year_issue_id": key})
	}
	return issues, nil
}
func issueNumber(key, label string) string {
	if len(key) > 4 && utf8.ValidString(key[:4]) {
		value := strings.TrimLeft(key[4:], "0")
		if value == "" {
			return "0"
		}
		return value
	}
	var digits strings.Builder
	for _, character := range label {
		if character >= '0' && character <= '9' {
			digits.WriteRune(character)
		}
	}
	if digits.Len() == 0 {
		return label
	}
	return digits.String()
}

// ParseIssueArticles validates each page's exact count and ten-row continuation rule.
func ParseIssueArticles(text string, issue any, pageIndex uint64) (IssueArticlePage, error) {
	isEmpty := inputValue(text, "articleCount") == nil && !strings.Contains(text, `class="row clearfix`) && (explicitEmpty(text) || pageIndex > 0 && updatePlaceholder(text))
	if isEmpty {
		if err := CheckedText(text); err != nil {
			return IssueArticlePage{}, err
		}
	} else if err := validateResponse("issue_articles", text); err != nil {
		return IssueArticlePage{}, err
	}
	var count uint64
	if value := inputValue(text, "articleCount"); value != nil {
		parsed, err := strconv.ParseUint(strings.TrimPrefix(*value, "+"), 10, 64)
		if err != nil {
			return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic issue article page has invalid articleCount"}
		}
		count = parsed
	} else if !isEmpty {
		return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic issue article page missing articleCount"}
	}
	articles := []any{}
	section := ""
	cursor := 0
	for cursor < len(text) {
		first, second := strings.Index(text[cursor:], "<dt"), strings.Index(text[cursor:], "<dd")
		if first < 0 && second < 0 {
			break
		}
		name := "dd"
		start := second
		if first >= 0 && (second < 0 || first <= second) {
			name = "dt"
			start = first
		}
		block, end, ok := tagBlockAt(text, name, cursor+start)
		if !ok {
			break
		}
		cursor = end
		if name == "dt" {
			section = stripTags(block)
			continue
		}
		article, err := parseArticleRow(block, issue, section)
		if err != nil {
			return IssueArticlePage{}, err
		}
		if article != nil {
			articles = append(articles, article)
		}
	}
	if uint64(len(articles)) != count {
		return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic issue article count does not match parsed rows"}
	}
	return IssueArticlePage{Articles: articles, PageIndex: pageIndex, ArticleCount: count, HasNextPage: count == 10}, nil
}
func parseArticleRow(text string, issue any, section string) (any, error) {
	for _, tag := range tags(text, "a") {
		values := attrs(tag)
		href := values["href"]
		if !strings.Contains(href, "/kcms2/article/abstract") && !strings.Contains(href, "/article/abstract") {
			continue
		}
		title := firstValue(nonEmpty(values["title"]), nonEmpty(stripTags(tag)))
		if title == nil {
			continue
		}
		url, err := WithDomesticPlatform(href)
		if err != nil {
			return nil, err
		}
		if ContainsOverseasHost(url) {
			return nil, &Error{Kind: "Parse", Message: "domestic issue article returned overseas host"}
		}
		var platformId *string
		for _, bold := range tags(text, "b") {
			attributes := attrs(bold)
			if attributes["name"] == "encrypt" {
				platformId = nonEmpty(attributes["id"])
				if platformId != nil {
					break
				}
			}
		}
		return map[string]any{"title": *title, "article_url": url, "platform_id": optional(platformId), "authors": optional(spanTitle(text, "author")), "pages": optional(spanTitle(text, "company")), "section": optional(nonEmpty(section)), "year": cloneJson(field(issue, "year")), "number": cloneJson(field(issue, "number")), "platform": "NZKPT"}, nil
	}
	return nil, nil
}

// ParseArticleDetail validates the document before its URL and preserves legacy fallbacks.
func ParseArticleDetail(text, articleUrl string) (any, error) {
	if err := validateResponse("article_detail", text); err != nil {
		return nil, err
	}
	parsed, err := parseDomesticUrl(articleUrl)
	if err != nil {
		return nil, err
	}
	permalink, err := WithDomesticPlatform(parsed.Href(false))
	if err != nil {
		return nil, err
	}
	abstract := firstValue(inputValue(text, "abstract_text"), summaryText(text))
	if abstract != nil {
		abstract = nonEmpty(stripTags(decodeHtml(*abstract)))
	}
	online := firstValue(rowValue(text, "在线公开时间"), rowValue(text, "Online Release Time"))
	if online != nil {
		parts := strings.Fields(*online)
		online = nil
		if len(parts) > 0 {
			online = nonEmpty(parts[0])
		}
	}
	return map[string]any{"article_url": permalink, "platform_id": optional(firstValue(inputValue(text, "paramfilename"), inputValue(text, "param-filename"))), "dbcode": optional(firstValue(inputValue(text, "paramdbcode"), inputValue(text, "param-dbcode"))), "dbname": optional(firstValue(inputValue(text, "paramdbname"), inputValue(text, "param-dbname"))), "title": optional(firstValue(firstBlockText(text, "h1", "title"), firstBlockText(text, "p", "title-one"), titleText(text))), "authors": optional(firstValue(authorText(text), spanTitle(text, "author"))), "abstract": optional(abstract), "doi": optional(rowValue(text, "DOI")), "online_release_date": optional(online), "pages": optional(labelValue(stripTags(text), "页码", "Pages")), "permalink": permalink, "platform": "NZKPT"}, nil
}
