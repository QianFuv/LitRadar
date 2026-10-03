package cnki

import (
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"golang.org/x/net/html"
	"golang.org/x/net/html/atom"
)

func decodeHtml(value string) string {
	for _, pair := range [][2]string{{"&amp;", "&"}, {"&lt;", "<"}, {"&gt;", ">"}, {"&quot;", "\""}, {"&#39;", "'"}, {"&nbsp;", " "}} {
		value = strings.ReplaceAll(value, pair[0], pair[1])
	}
	return value
}
func nonEmpty(value string) *string {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	return &value
}
func cleanText(value string) *string {
	return nonEmpty(strings.ReplaceAll(decodeHtml(value), "\u00a0", " "))
}
func stripTags(value string) string {
	var output strings.Builder
	isInTag := false
	for _, character := range value {
		switch character {
		case '<':
			isInTag = true
		case '>':
			isInTag = false
		default:
			if !isInTag {
				output.WriteRune(character)
			}
		}
	}
	return decodeHtml(output.String())
}
func tagBlockAt(text, name string, start int) (string, int, bool) {
	opening := strings.IndexByte(text[start:], '>')
	if opening < 0 {
		return "", 0, false
	}
	opening += start + 1
	marker := "</" + name + ">"
	closing := strings.Index(text[opening:], marker)
	if closing < 0 {
		return "", 0, false
	}
	end := opening + closing + len(marker)
	return text[start:end], end, true
}
func findTagBlock(text, name string, from int) (string, int, bool) {
	start := strings.Index(text[from:], "<"+name)
	if start < 0 {
		return "", 0, false
	}
	return tagBlockAt(text, name, from+start)
}
func tags(text, name string) []string {
	result := []string{}
	cursor := 0
	for {
		block, end, ok := findTagBlock(text, name, cursor)
		if !ok {
			break
		}
		result = append(result, block)
		cursor = end
	}
	return result
}
func startTags(text, name string) []string {
	result := []string{}
	cursor := 0
	for {
		start := strings.Index(text[cursor:], "<"+name)
		if start < 0 {
			break
		}
		start += cursor
		end := strings.IndexByte(text[start:], '>')
		if end < 0 {
			break
		}
		cursor = start + end + 1
		result = append(result, text[start:cursor])
	}
	return result
}
func attrs(tag string) map[string]string {
	header, _, _ := strings.Cut(tag, ">")
	result := map[string]string{}
	for _, quote := range []byte{'"', '\''} {
		cursor := 0
		for cursor < len(header) {
			equals := strings.IndexByte(header[cursor:], '=')
			if equals < 0 {
				break
			}
			equals += cursor
			if equals+1 >= len(header) || header[equals+1] != quote {
				cursor = equals + 1
				continue
			}
			keyStart := 0
			for index, character := range header[:equals] {
				if unicode.IsSpace(character) || character == '<' {
					keyStart = index + len(string(character))
				}
			}
			key := domain.Lowercase(strings.TrimSpace(header[keyStart:equals]))
			valueStart := equals + 2
			valueEnd := strings.IndexByte(header[valueStart:], quote)
			if valueEnd < 0 {
				break
			}
			valueEnd += valueStart
			if key != "" {
				result[key] = decodeHtml(header[valueStart:valueEnd])
			}
			cursor = valueEnd + 1
		}
	}
	return result
}
func inputValue(text, id string) *string {
	for _, tag := range startTags(text, "input") {
		values := attrs(tag)
		if values["id"] == id {
			if value := nonEmpty(values["value"]); value != nil {
				return value
			}
		}
	}
	return nil
}
func hasClass(classes, want string) bool {
	for _, value := range strings.Fields(classes) {
		if value == want {
			return true
		}
	}
	return false
}
func firstBlockText(text, name, class string) *string {
	for _, tag := range tags(text, name) {
		if hasClass(attrs(tag)["class"], class) {
			if value := nonEmpty(stripTags(tag)); value != nil {
				return value
			}
		}
	}
	return nil
}
func titleText(text string) *string {
	block, _, ok := findTagBlock(text, "title", 0)
	if !ok {
		return nil
	}
	return nonEmpty(strings.ReplaceAll(strings.ReplaceAll(stripTags(block), " - 中国知网", ""), "-中国知网", ""))
}
func labelValue(text string, labels ...string) *string {
	for _, label := range labels {
		for _, separator := range []string{":", "："} {
			marker := label + separator
			index := strings.Index(text, marker)
			if index < 0 {
				continue
			}
			rest := strings.TrimLeftFunc(text[index+len(marker):], unicode.IsSpace)
			end := strings.IndexFunc(rest, func(character rune) bool { return unicode.IsSpace(character) || character == '；' })
			if end >= 0 {
				rest = rest[:end]
			}
			if value := nonEmpty(rest); value != nil {
				return value
			}
		}
	}
	return nil
}
func rowValue(text, label string) *string {
	cursor := 0
	for {
		start := strings.Index(text[cursor:], "<span")
		if start < 0 {
			break
		}
		block, end, ok := tagBlockAt(text, "span", cursor+start)
		if !ok {
			break
		}
		if hasClass(attrs(block)["class"], "rowtit") && strings.TrimSpace(strings.TrimRight(strings.TrimSpace(stripTags(block)), ":：")) == label {
			if paragraph, _, ok := findTagBlock(text, "p", end); ok {
				return nonEmpty(stripTags(paragraph))
			}
			if span, _, ok := findTagBlock(text, "span", end); ok {
				return nonEmpty(stripTags(span))
			}
		}
		cursor = end
	}
	return nil
}
func summaryText(text string) *string {
	for _, tag := range tags(text, "span") {
		values := attrs(tag)
		if values["id"] == "ChDivSummary" || hasClass(values["class"], "abstract-text") {
			if value := nonEmpty(stripTags(tag)); value != nil {
				return value
			}
		}
	}
	return inputValue(text, "abstract_text")
}
func spanTitle(text, class string) *string {
	if class == "author" {
		for _, node := range authorNodes(text) {
			if node.Data == "span" && htmlHasClass(node, "author") {
				if title := nonEmpty(htmlAttribute(node, "title")); title != nil {
					return title
				}
				if value := authorElementText(node); value != nil {
					return value
				}
			}
		}
		return nil
	}
	for _, tag := range tags(text, "span") {
		values := attrs(tag)
		if hasClass(values["class"], class) {
			if value := cleanText(values["title"]); value != nil {
				return value
			}
			if value := nonEmpty(stripTags(tag)); value != nil {
				return value
			}
		}
	}
	return nil
}
func authorNodes(text string) []*html.Node {
	roots, err := html.ParseFragment(strings.NewReader(text), &html.Node{Type: html.ElementNode, Data: "body", DataAtom: atom.Body})
	if err != nil {
		return nil
	}
	nodes := []*html.Node{}
	var visit func(*html.Node)
	visit = func(node *html.Node) {
		if node.Type == html.ElementNode {
			nodes = append(nodes, node)
		}
		for child := node.FirstChild; child != nil; child = child.NextSibling {
			visit(child)
		}
	}
	for _, root := range roots {
		visit(root)
	}
	orderAuthorNodes(text, roots, nodes)
	return nodes
}
func htmlAttribute(node *html.Node, name string) string {
	for _, attribute := range node.Attr {
		if attribute.Key == name {
			return attribute.Val
		}
	}
	return ""
}
func htmlHasClass(node *html.Node, class string) bool {
	for _, value := range strings.FieldsFunc(htmlAttribute(node, "class"), func(character rune) bool { return strings.ContainsRune(" \t\n\r\f", character) }) {
		if value == class {
			return true
		}
	}
	return false
}
func authorElementText(node *html.Node) *string {
	var output strings.Builder
	var appendText func(*html.Node)
	appendText = func(parent *html.Node) {
		for child := parent.FirstChild; child != nil; child = child.NextSibling {
			if child.Type == html.TextNode {
				output.WriteString(child.Data)
			} else if child.Type == html.ElementNode && child.Data != "sup" && child.Data != "script" && child.Data != "style" && !(child.Data == "template" && child.Namespace == "") {
				appendText(child)
			}
		}
	}
	appendText(node)
	return nonEmpty(strings.Join(strings.Fields(output.String()), " "))
}
func authorText(text string) *string {
	for _, block := range authorNodes(text) {
		if block.Data != "h3" || !htmlHasClass(block, "author") || htmlAttribute(block, "id") != "authorpart" {
			continue
		}
		names := []string{}
		var visit func(*html.Node, bool)
		visit = func(parent *html.Node, isBlocked bool) {
			for child := parent.FirstChild; child != nil; child = child.NextSibling {
				if child.Type != html.ElementNode {
					continue
				}
				if child.Data == "span" && !isBlocked {
					if name := authorElementText(child); name != nil {
						names = append(names, *name)
					}
				}
				visit(child, isBlocked || child.Data == "span" || child.Data == "sup")
			}
		}
		visit(block, false)
		if len(names) == 0 {
			return nil
		}
		result := strings.Join(names, "; ")
		return &result
	}
	return nil
}
