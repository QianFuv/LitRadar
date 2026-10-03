package cnki

import (
	"fmt"
	"slices"
	"strconv"
	"strings"

	"golang.org/x/net/html"
	"golang.org/x/net/html/atom"
)

// orderAuthorNodes restores scraper's global node-creation order after HTML foster parenting.
// Extraction continues to use the original DOM; descendant traversal remains DOM ordered.
func orderAuthorNodes(text string, roots, nodes []*html.Node) {
	candidates := []*html.Node{}
	for _, node := range nodes {
		if node.Data == "span" && htmlHasClass(node, "author") || node.Data == "h3" && htmlHasClass(node, "author") && htmlAttribute(node, "id") == "authorpart" {
			candidates = append(candidates, node)
		}
	}
	if len(candidates) < 2 {
		return
	}
	marker := "data-litradar-source-order"
	lowered := strings.ToLower(text)
	for strings.Contains(lowered, marker) || authorTreeContains(roots, marker) {
		marker += "x"
	}
	ranks := map[*html.Node]int{}
	merge := func(marked string) {
		shadow, err := html.ParseFragment(strings.NewReader(marked), &html.Node{Type: html.ElementNode, Data: "body", DataAtom: atom.Body})
		if err != nil || len(roots) != len(shadow) {
			return
		}
		found := map[*html.Node]int{}
		for index, root := range roots {
			if !alignAuthorTree(root, shadow[index], marker, found) {
				return
			}
		}
		for node, rank := range found {
			ranks[node] = rank
		}
	}
	for _, isRawDisabled := range []bool{false, true} {
		merge(markAuthorTokens(text, marker, isRawDisabled))
		if authorRanksComplete(candidates, ranks) {
			break
		}
	}
	if !authorRanksComplete(candidates, ranks) {
		for offset := 0; offset < len(text); offset++ {
			if text[offset] != '<' {
				continue
			}
			length := authorTagNameLength(text[offset:])
			if length == 0 {
				continue
			}
			insertion := offset + 1 + length
			merge(text[:insertion] + fmt.Sprintf(` %s="%d"`, marker, offset) + text[insertion:])
			if authorRanksComplete(candidates, ranks) {
				break
			}
		}
	}
	positions := []int{}
	ordered := []*html.Node{}
	for index, node := range nodes {
		if _, exists := ranks[node]; exists {
			positions = append(positions, index)
			ordered = append(ordered, node)
		}
	}
	slices.SortStableFunc(ordered, func(first, second *html.Node) int { return ranks[first] - ranks[second] })
	for index, position := range positions {
		nodes[position] = ordered[index]
	}
}
func authorTreeContains(roots []*html.Node, marker string) bool {
	var contains func(*html.Node) bool
	contains = func(node *html.Node) bool {
		if strings.Contains(strings.ToLower(node.Data), marker) {
			return true
		}
		for _, attribute := range node.Attr {
			if strings.Contains(strings.ToLower(attribute.Key), marker) || strings.Contains(strings.ToLower(attribute.Val), marker) {
				return true
			}
		}
		for child := node.FirstChild; child != nil; child = child.NextSibling {
			if contains(child) {
				return true
			}
		}
		return false
	}
	for _, root := range roots {
		if contains(root) {
			return true
		}
	}
	return false
}
func authorRanksComplete(nodes []*html.Node, ranks map[*html.Node]int) bool {
	for _, node := range nodes {
		if _, exists := ranks[node]; !exists {
			return false
		}
	}
	return true
}
func authorTagNameLength(raw string) int {
	for _, name := range []string{"span", "h3"} {
		end := 1 + len(name)
		if len(raw) <= end || raw[0] != '<' || asciiLower(raw[1:end]) != name {
			continue
		}
		if strings.ContainsRune(" \t\r\n\f/>", rune(raw[end])) {
			return len(name)
		}
	}
	return 0
}
func markAuthorTokens(text, marker string, isRawDisabled bool) string {
	var output strings.Builder
	output.Grow(len(text))
	tokenizer := html.NewTokenizer(strings.NewReader(text))
	offset := 0
	for {
		kind := tokenizer.Next()
		raw := string(tokenizer.Raw())
		if kind == html.StartTagToken || kind == html.SelfClosingTagToken {
			if length := authorTagNameLength(raw); length > 0 {
				insertion := length + 1
				output.WriteString(raw[:insertion])
				fmt.Fprintf(&output, ` %s="%d"`, marker, offset)
				output.WriteString(raw[insertion:])
			} else {
				output.WriteString(raw)
			}
			if isRawDisabled {
				tokenizer.NextIsNotRawText()
			}
		} else {
			output.WriteString(raw)
		}
		offset += len(raw)
		if kind == html.ErrorToken {
			break
		}
	}
	return output.String()
}
func alignAuthorTree(original, shadow *html.Node, marker string, ranks map[*html.Node]int) bool {
	if original.Type != shadow.Type || original.Namespace != shadow.Namespace {
		return false
	}
	if original.Type == html.TextNode || original.Type == html.CommentNode {
		if original.Data != removeOrderMarkers(shadow.Data, marker) {
			return false
		}
	} else if original.Data != shadow.Data {
		return false
	}
	attributes := make([]html.Attribute, 0, len(shadow.Attr))
	for _, attribute := range shadow.Attr {
		if attribute.Key == marker && attribute.Namespace == "" {
			rank, err := strconv.Atoi(attribute.Val)
			if err != nil {
				return false
			}
			ranks[original] = rank
		} else {
			attribute.Val = removeOrderMarkers(attribute.Val, marker)
			attributes = append(attributes, attribute)
		}
	}
	if !slices.Equal(original.Attr, attributes) {
		return false
	}
	first, second := original.FirstChild, shadow.FirstChild
	for first != nil && second != nil {
		if !alignAuthorTree(first, second, marker, ranks) {
			return false
		}
		first, second = first.NextSibling, second.NextSibling
	}
	return first == nil && second == nil
}
func removeOrderMarkers(text, marker string) string {
	prefix := " " + marker + `="`
	for {
		start := strings.Index(text, prefix)
		if start < 0 {
			return text
		}
		end := strings.IndexByte(text[start+len(prefix):], '"')
		if end < 0 {
			return text
		}
		end += start + len(prefix)
		if _, err := strconv.Atoi(text[start+len(prefix) : end]); err != nil {
			return text
		}
		text = text[:start] + text[end+1:]
	}
}
