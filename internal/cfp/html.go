package cfp

import (
	"regexp"
	"slices"
	"strings"
	"unicode"

	"github.com/PuerkitoBio/goquery"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"golang.org/x/net/html"
	"golang.org/x/net/html/atom"
	"golang.org/x/text/cases"
	"golang.org/x/text/language"
)

const headingPattern = `(?m)^(#{1,6}) (.+)$`
const statusPattern = `(?i)(?:Submission status|Status)[\s:]+(Open(?: for submissions)?|Closed|Upcoming)`
const dateLabelPattern = `(?i)deadline|submissions? (?:due|close|open|window|period)|(?:full )?(?:paper|manuscript|abstract|proposal) (?:submission|due)|(?:submit|submission) (?:by|before|until|deadline)|notification|final decision|camera.ready|revised manuscript|截[稿止]|投稿时间|征稿时间|来稿时间|提交时间|论文.*(?:发送|提交)|(?:发送|提交).*论文`
const dateTokenPattern = `(?i)20\d{2}|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\w*\b|\d+月|ongoing|rolling|TBD|to be (?:announced|determined)|待定`

func pattern(pattern, text string) bool { return len(domain.PatternCaptures(pattern, text)) > 0 }
func parseHtml(text string) *goquery.Document {
	node, err := html.Parse(strings.NewReader(text))
	if err != nil {
		panic(err)
	}
	return goquery.NewDocumentFromNode(node)
}
func fragment(text string) *goquery.Document {
	root := &html.Node{Type: html.ElementNode, Data: "div", DataAtom: atom.Div}
	nodes, err := html.ParseFragment(strings.NewReader(text), root)
	if err != nil {
		panic(err)
	}
	for _, node := range nodes {
		root.AppendChild(node)
	}
	return goquery.NewDocumentFromNode(root)
}
func inner(element *goquery.Selection) string {
	value, err := element.Html()
	if err != nil {
		return ""
	}
	return value
}
func attribute(node *html.Node, name string) (string, bool) {
	for _, attribute := range node.Attr {
		if attribute.Key == name {
			return attribute.Val, true
		}
	}
	return "", false
}

// visible preserves the original DOM traversal's paragraph/heading boundaries and hidden exclusions.
func visible(document *goquery.Document, exclusions string) string {
	excluded := map[*html.Node]bool{}
	if exclusions != "" {
		for _, node := range document.Find(exclusions).Nodes {
			excluded[node] = true
		}
	}
	var result strings.Builder
	var walk func(*html.Node)
	walk = func(node *html.Node) {
		if excluded[node] {
			return
		}
		if node.Type == html.TextNode {
			result.WriteString(node.Data)
			return
		}
		if node.Type == html.DocumentNode {
			for child := node.FirstChild; child != nil; child = child.NextSibling {
				walk(child)
			}
			return
		}
		if node.Type != html.ElementNode {
			return
		}
		name := node.Data
		_, isHidden := attribute(node, "hidden")
		aria, _ := attribute(node, "aria-hidden")
		if slices.Contains([]string{"script", "style", "head", "noscript", "svg", "nav", "footer"}, name) || isHidden || aria == "true" {
			return
		}
		isHeading := len(name) == 2 && name[0] == 'h' && name[1] >= '1' && name[1] <= '6'
		isBlock := isHeading || slices.Contains([]string{"p", "div", "section", "article", "li", "tr", "br", "dl", "dt", "dd"}, name)
		if isBlock {
			result.WriteByte('\n')
		}
		if isHeading {
			result.WriteString(strings.Repeat("#", int(name[1]-'0')) + " ")
		}
		for child := node.FirstChild; child != nil; child = child.NextSibling {
			walk(child)
		}
		if isBlock {
			result.WriteByte('\n')
		}
	}
	for _, node := range document.Nodes {
		walk(node)
	}
	lines := []string{}
	for _, line := range strings.Split(domain.CleanText(result.String()), "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			lines = append(lines, line)
		}
	}
	return strings.Join(lines, "\n")
}
func plain(value string) string {
	lines := strings.Split(value, "\n")
	for index, line := range lines {
		lines[index] = strings.TrimSpace(strings.TrimLeft(line, "#"))
	}
	return domain.CleanText(strings.Join(lines, "\n"))
}
func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}
func equalAscii(first, second string) bool { return asciiLower(first) == asciiLower(second) }
func lower(value string) string            { return cases.Lower(language.Und).String(value) }
func matchingTitle(first, second string) bool {
	normalize := func(value string) string {
		return strings.ToLower(strings.ReplaceAll(strings.Map(func(character rune) rune {
			if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
				return character
			}
			return -1
		}, domain.CleanText(value)), "İ", "i\u0307"))
	}
	first = normalize(first)
	return first != "" && first == normalize(second)
}

// IsChallenge checks publisher headings plus known script challenge shells.
func IsChallenge(document Document) bool {
	parsed := parseHtml(document.Text)
	titles := []string{}
	for _, node := range parsed.Find("title,h1").Nodes {
		titles = append(titles, goquery.NewDocumentFromNode(node).Text())
	}
	if pattern(`(?i)just a moment|client challenge|access denied|verify (?:that )?you are human|captcha|robot verification`, strings.Join(titles, "\n")) {
		return true
	}
	hasScripts := pattern(`(?i)cf-chl-|challenge-platform|enable javascript and cookies to continue|/_fs-ch-`, document.Text)
	text := visible(parsed, "")
	return hasScripts && (strings.TrimSpace(text) == "" || pattern(`(?im)^(?:#{1,6} )?(?:performing security verification|verifying you are human|enable javascript and cookies to continue|checking your browser before accessing|verification successful\. waiting for .+ to respond)[.!]?\s*$`, text))
}

type headingBlock struct{ title, body string }

func blocks(text string, minimum, maximum int) []headingBlock {
	headings := domain.PatternCaptures(headingPattern, text)
	result := []headingBlock{}
	for index, heading := range headings {
		level := heading[3] - heading[2]
		if level < minimum || level > maximum {
			continue
		}
		end := len(text)
		for _, later := range headings[index+1:] {
			if later[3]-later[2] <= level {
				end = later[0]
				break
			}
		}
		result = append(result, headingBlock{plain(text[heading[4]:heading[5]]), strings.TrimSpace(text[heading[1]:end])})
	}
	return result
}
func removePattern(expression, text string) string {
	var result strings.Builder
	previous := 0
	for _, match := range domain.PatternCaptures(expression, text) {
		result.WriteString(text[previous:match[0]])
		previous = match[1]
	}
	result.WriteString(text[previous:])
	return result.String()
}
func dateClauses(body string) string {
	text := plain(body)
	text = regexp.MustCompile(`([.!?])[\s\p{Z}\x{85}]+([A-Z])`).ReplaceAllString(text, "$1\n$2")
	lines := strings.Split(text, "\n")
	clauses := []string{}
	for index, line := range lines {
		if !pattern(dateLabelPattern, line) {
			continue
		}
		clause := line
		if !pattern(dateTokenPattern, line) {
			if index+1 >= len(lines) || !pattern(dateTokenPattern, lines[index+1]) {
				continue
			}
			clause += " " + lines[index+1]
		}
		if !slices.Contains(clauses, clause) {
			clauses = append(clauses, clause)
		}
	}
	return strings.Join(clauses, "\n")
}

func record(config SourceConfig, document Document, checkedOn, title, body, status string) (domain.Source, error) {
	typeText := "Special Issue"
	stage := domain.Paper
	if pattern(`(?i)proposals?|提案`, title) {
		typeText = "Call for Proposals"
		stage = domain.Proposal
	}
	dateText := dateClauses(removePattern(statusPattern, plain(body)))
	rawDateText := ""
	if domain.ParseDates(dateText, stage) == nil {
		rawDateText, dateText = dateText, ""
	}
	requirements := ""
	for _, block := range blocks(body, 2, 6) {
		if pattern(`(?i)submission (?:instructions|guidelines)|manuscript requirements|投稿要求|稿件要求`, block.title) {
			requirements = plain(block.body)
			break
		}
	}
	scopeLines := []string{}
	for _, line := range strings.Split(plain(body), "\n") {
		if pattern(dateLabelPattern, line) || pattern(statusPattern, line) || pattern(`(?i)^guest editors?|^submission (?:instructions|guidelines)|^manuscript requirements|^投稿要求|^稿件要求`, line) {
			break
		}
		if !pattern(`(?i)^(?:[0-3]?\d\s+[a-z]{3,9}\s+20\d{2}|[a-z]{3,9}\s+[0-3]?\d,?\s+20\d{2}|20\d{2}[-/]\d{1,2}[-/]\d{1,2})$`, strings.TrimSpace(line)) {
			scopeLines = append(scopeLines, line)
		}
	}
	for _, line := range strings.Split(plain(title+"\n"+body), "\n") {
		if pattern(`(?i)invit(?:ation|e|ed)[ -]*only|invitation required|invited (?:submissions|papers) only|仅限受邀`, line) {
			status += "\n" + line
			break
		}
	}
	var statusText *string
	if status != "" {
		statusText = &status
	}
	source := domain.Source{CatalogIds: append([]string{}, config.CatalogIds...), JournalTitle: config.JournalTitle, Title: plain(title), Scope: plain(strings.Join(scopeLines, "\n")), Requirements: requirements, TypeText: typeText, DateText: dateText, SourceUrl: document.FinalUrl, CheckedOn: checkedOn, StatusText: statusText, RawDateText: rawDateText}
	if domain.ParseSource(source) == nil {
		return domain.Source{}, ErrUnrecognized
	}
	return source, nil
}

func hasJournalIdentity(config SourceConfig, document *goquery.Document, text string) bool {
	firstCard := len(text)
	level := 3
	if config.Adapter == SpringerCollections {
		level = 2
	}
	for _, heading := range domain.PatternCaptures(headingPattern, text) {
		if heading[3]-heading[2] >= level {
			firstCard = heading[0]
			break
		}
	}
	prefix := text[:firstCard]
	titles := []string{}
	for _, node := range document.Find("title").Nodes {
		titles = append(titles, goquery.NewDocumentFromNode(node).Text())
	}
	identityText := strings.Join(titles, "\n") + "\n" + prefix
	for _, identity := range config.IdentityTexts {
		if pattern(`^\d{4}-\d{3}[0-9X]$`, identity) {
			if pattern(`(?i)\b(?:e-?)?issn\s*[:：]?\s*`+regexp.QuoteMeta(identity)+`\b`, prefix) {
				return true
			}
			continue
		}
		segments := strings.Split(identityText, "\n")
		for _, separator := range []string{" | ", " - ", " — "} {
			next := []string{}
			for _, segment := range segments {
				next = append(next, strings.Split(segment, separator)...)
			}
			segments = next
		}
		for _, line := range segments {
			if equalAscii(strings.TrimSpace(strings.TrimLeft(line, "#")), identity) {
				return true
			}
		}
	}
	return false
}
