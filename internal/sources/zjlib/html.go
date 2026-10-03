package zjlib

import (
	"strings"
	"unicode"
)

func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}
func attrValue(tag, name string) *string {
	start := strings.Index(asciiLower(tag), name+"=")
	if start < 0 {
		return nil
	}
	rest := strings.TrimLeftFunc(tag[start+len(name)+1:], unicode.IsSpace)
	if rest == "" {
		return nil
	}
	var value string
	if rest[0] == '\'' || rest[0] == '"' {
		value = strings.SplitN(rest[1:], rest[:1], 2)[0]
	} else {
		value = strings.Fields(rest)[0]
	}
	value = strings.TrimSpace(decodeHtml(value))
	if value == "" {
		return nil
	}
	return &value
}

type anchorLink struct {
	href, body string
	start, end int
}

func anchorLinks(text string) []anchorLink {
	lower := asciiLower(text)
	links := []anchorLink{}
	for cursor := 0; cursor < len(text); {
		start := strings.Index(lower[cursor:], "<a")
		if start < 0 {
			break
		}
		start += cursor
		tagEnd := strings.IndexByte(lower[start:], '>')
		if tagEnd < 0 {
			break
		}
		tagEnd += start + 1
		href := attrValue(text[start:tagEnd], "href")
		cursor = tagEnd
		if href == nil {
			continue
		}
		closeAt := strings.Index(lower[tagEnd:], "</a>")
		if closeAt < 0 {
			continue
		}
		closeAt += tagEnd
		links = append(links, anchorLink{href: *href, body: text[tagEnd:closeAt], start: start, end: closeAt + 4})
		cursor = closeAt + 4
	}
	return links
}
func anchorTitle(body string) *string {
	const marker = "ReplaceJiankuohao('"
	if start := strings.Index(body, marker); start >= 0 {
		start += len(marker)
		if end := strings.Index(body[start:], "')"); end >= 0 {
			return cleanText(stripTags(body[start : start+end]))
		}
	}
	return cleanText(stripTags(body))
}
func metaContentList(text, name string) []string {
	lower := asciiLower(text)
	values := []string{}
	for cursor := 0; cursor < len(text); {
		start := strings.Index(lower[cursor:], "<meta")
		if start < 0 {
			break
		}
		start += cursor
		end := strings.IndexByte(lower[start:], '>')
		if end < 0 {
			break
		}
		end += start + 1
		tag := text[start:end]
		cursor = end
		if tagName := attrValue(tag, "name"); tagName != nil && asciiLower(*tagName) == asciiLower(name) {
			if content := attrValue(tag, "content"); content != nil {
				if cleaned := cleanText(*content); cleaned != nil {
					values = append(values, *cleaned)
				}
			}
		}
	}
	return values
}
func metaContent(text, name string) *string {
	values := metaContentList(text, name)
	if len(values) > 0 {
		return &values[0]
	}
	return nil
}
func firstTagText(text, name string) *string {
	lower := asciiLower(text)
	start := strings.Index(lower, "<"+name)
	if start < 0 {
		return nil
	}
	openEnd := strings.IndexByte(lower[start:], '>')
	if openEnd < 0 {
		return nil
	}
	openEnd += start + 1
	closeAt := strings.Index(lower[openEnd:], "</"+name+">")
	if closeAt < 0 {
		return nil
	}
	return cleanText(stripTags(text[openEnd : openEnd+closeAt]))
}
func titleText(text string) *string {
	title := firstTagText(text, "title")
	if title == nil {
		return nil
	}
	output := *title
	for _, suffix := range []string{" - 中国知网", " - CNKI", " - 中国学术期刊网络出版总库"} {
		if strings.HasSuffix(output, suffix) {
			output = strings.TrimSpace(strings.TrimSuffix(output, suffix))
		}
	}
	return cleanText(output)
}
func spanTexts(text string) []string {
	lower := asciiLower(text)
	output := []string{}
	for cursor := 0; cursor < len(text); {
		start := strings.Index(lower[cursor:], "<span")
		if start < 0 {
			break
		}
		start += cursor
		openEnd := strings.IndexByte(lower[start:], '>')
		if openEnd < 0 {
			break
		}
		openEnd += start + 1
		closeAt := strings.Index(lower[openEnd:], "</span>")
		if closeAt < 0 {
			break
		}
		closeAt += openEnd
		if value := cleanText(stripTags(text[openEnd:closeAt])); value != nil {
			output = append(output, *value)
		}
		cursor = closeAt + 7
	}
	return output
}
func authorBlockText(text string) *string {
	lower := asciiLower(text)
	idStart := strings.Index(lower, `id="authorpart"`)
	if idStart < 0 {
		return nil
	}
	blockStart := strings.LastIndex(lower[:idStart], "<h3")
	if blockStart < 0 {
		blockStart = idStart
	}
	blockEnd := strings.Index(lower[idStart:], "</h3>")
	if blockEnd < 0 {
		return nil
	}
	blockEnd += idStart + 5
	block := text[blockStart:blockEnd]
	names := spanTexts(block)
	if len(names) == 0 {
		return cleanText(stripTags(block))
	}
	return pointer(strings.Join(names, "; "))
}
func labelBlock(text, label string) *string {
	marker := "【" + label + "】"
	start := strings.Index(text, marker)
	if start < 0 {
		return nil
	}
	rest := text[start+len(marker):]
	end := len(rest)
	for _, marker := range []string{"</p>", "</li>", "</div>"} {
		if at := strings.Index(rest, marker); at >= 0 && at < end {
			end = at
		}
	}
	return pointer(rest[:end])
}
func labelAuthors(text, label string) *string {
	block := labelBlock(text, label)
	if block == nil {
		return nil
	}
	names := []string{}
	for _, anchor := range anchorLinks(*block) {
		if name := cleanText(stripTags(anchor.body)); name != nil {
			names = append(names, *name)
		}
	}
	if len(names) == 0 {
		return nil
	}
	return pointer(strings.Join(names, "; "))
}
func labelFirstAnchor(text, label string) *string {
	block := labelBlock(text, label)
	if block == nil {
		return nil
	}
	for _, anchor := range anchorLinks(*block) {
		if value := cleanText(stripTags(anchor.body)); value != nil {
			return value
		}
	}
	return cleanText(stripTags(*block))
}
func labelSpanText(text, label, id string) *string {
	block := labelBlock(text, label)
	if block == nil {
		return nil
	}
	lower := asciiLower(*block)
	idStart := strings.Index(lower, `id="`+asciiLower(id)+`"`)
	if idStart < 0 {
		return nil
	}
	start := strings.LastIndex(lower[:idStart], "<span")
	if start < 0 {
		start = idStart
	}
	openEnd := strings.IndexByte(lower[start:], '>')
	if openEnd < 0 {
		return nil
	}
	openEnd += start + 1
	closeAt := strings.Index(lower[openEnd:], "</span>")
	if closeAt < 0 {
		return nil
	}
	return cleanText(stripTags((*block)[openEnd : openEnd+closeAt]))
}
func rowValue(text, label string) *string {
	plain := stripTags(text)
	marker := label + ":"
	start := strings.Index(plain, marker)
	if start < 0 {
		marker = label + "："
		start = strings.Index(plain, marker)
	}
	if start < 0 {
		return nil
	}
	rest := plain[start+len(marker):]
	if end := strings.IndexAny(rest, "\r\n"); end >= 0 {
		rest = rest[:end]
	}
	return cleanText(rest)
}
func firstValue(candidates ...func() *string) string {
	for _, candidate := range candidates {
		if value := candidate(); value != nil {
			return *value
		}
	}
	return ""
}

// ExtractArticleIdentity retains citation, legacy block and row fallback precedence.
func ExtractArticleIdentity(text, fallback string) ArticleIdentity {
	title := firstValue(func() *string { return metaContent(text, "citation_title") }, func() *string { return firstTagText(text, "h1") }, func() *string { return firstTagText(text, "h2") }, func() *string { return titleText(text) }, func() *string { return &fallback })
	authors := metaContentList(text, "citation_author")
	authorText := strings.Join(authors, "; ")
	if len(authors) == 0 {
		authorText = firstValue(func() *string { return authorBlockText(text) }, func() *string { return labelAuthors(text, "作者") }, func() *string { return rowValue(text, "作者") })
	}
	journal := firstValue(func() *string { return metaContent(text, "citation_journal_title") }, func() *string { return labelSpanText(text, "文献出处", "jname") }, func() *string { return labelFirstAnchor(text, "文献出处") }, func() *string { return labelFirstAnchor(text, "刊名") }, func() *string { return labelFirstAnchor(text, "来源") }, func() *string { return rowValue(text, "刊名") }, func() *string { return rowValue(text, "来源") })
	return ArticleIdentity{Title: title, Authors: authorText, JournalTitle: journal}
}
