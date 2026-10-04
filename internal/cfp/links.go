package cfp

import (
	"slices"
	"strings"

	"github.com/PuerkitoBio/goquery"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func hasTitle(selection *goquery.Selection, selector, title string) bool {
	for _, node := range selection.Find(selector).Nodes {
		if matchingTitle(goquery.NewDocumentFromNode(node).Text(), title) {
			return true
		}
	}
	return false
}
func springerUpdateBody(original domain.Source, document Document, parsed *goquery.Document) *goquery.Selection {
	location, err := whatwg.NewParser().Parse(document.FinalUrl)
	if err != nil || location.Hostname() != "link.springer.com" || !strings.HasPrefix(location.Pathname(), "/journal/10490/updates/") || !slices.Contains(original.CatalogIds, "issn-0217-4561") {
		return nil
	}
	body := parsed.Find("#updates-content-body").First()
	title := body.Find("h1").First()
	if body.Length() == 0 || title.Length() == 0 || !matchingTitle(visible(fragment(inner(title)), ".u-visually-hidden"), original.Title) {
		return nil
	}
	return body
}

// OriginalLinks binds recovery to the saved title and returns at most three list-navigation links.
func OriginalLinks(source domain.Source, document Document) []string {
	parsed := parseHtml(document.Text)
	base, err := whatwg.NewParser().Parse(document.FinalUrl)
	details := []string{}
	if err != nil {
		return details
	}
	join := func(node *goquery.Selection) *whatwg.Url {
		href, has := node.Attr("href")
		if !has {
			return nil
		}
		location, err := whatwg.NewParser().ParseRef(base.Href(false), href)
		if err != nil || (location.Scheme() != "http" && location.Scheme() != "https") {
			return nil
		}
		return location
	}
	if body := springerUpdateBody(source, document, parsed); body != nil {
		for _, node := range body.Find("p").Nodes {
			paragraph := goquery.NewDocumentFromNode(node)
			if !pattern(`(?i)read the full call for papers`, paragraph.Text()) {
				continue
			}
			for _, node := range paragraph.Find("a[href]").Nodes {
				if location := join(goquery.NewDocumentFromNode(node).Selection); location != nil {
					details = append(details, location.Href(false))
				}
			}
		}
		return details
	}
	cardSelector, titleSelector, linkSelector := "", "", ""
	isPdfOnly := false
	if base.Hostname() == "www.poms.org" && strings.TrimRight(base.Pathname(), "/") == "/journal/announcements" && slices.Contains(source.CatalogIds, "issn-1059-1478") {
		cardSelector = ".poms-special-issues"
		titleSelector = ".views-field-title .field-content"
		linkSelector = ".views-field-field-submission-guidelines-docu a[href], .views-field-views-conditional-field a[href]"
	}
	if base.Hostname() == "www.ieee-ras.org" && strings.TrimRight(base.Pathname(), "/") == "/publications/t-ase/special-issues-t-ase" && slices.Contains(source.CatalogIds, "issn-1545-5955") {
		cardSelector = "div[data-elementor-type='container'][data-elementor-id='16977']"
		titleSelector = "h3.dynamic-content-for-elementor-acf"
		linkSelector = "a.elementor-button[href]"
		isPdfOnly = true
	}
	if cardSelector != "" {
		for _, node := range parsed.Find(cardSelector).Nodes {
			card := goquery.NewDocumentFromNode(node)
			if !hasTitle(card.Selection, titleSelector, source.Title) {
				continue
			}
			for _, node := range card.Find(linkSelector).Nodes {
				location := join(goquery.NewDocumentFromNode(node).Selection)
				if location != nil && location.Hostname() == base.Hostname() && (!isPdfOnly || strings.HasSuffix(asciiLower(location.Pathname()), ".pdf")) && !slices.Contains(details, location.Href(false)) {
					details = append(details, location.Href(false))
				}
			}
		}
		return details
	}
	indexes := []string{}
	hasOriginalTitle := hasTitle(parsed.Selection, "h1,h2,h3,h4,title", source.Title)
	for _, node := range parsed.Find("a[href]").Nodes {
		element := goquery.NewDocumentFromNode(node)
		label := domain.CleanText(element.Text())
		location := join(element.Selection)
		if location == nil || location.Href(false) == base.Href(false) {
			continue
		}
		if matchingTitle(label, source.Title) || hasOriginalTitle && strings.HasSuffix(asciiLower(location.Pathname()), ".pdf") && pattern(`(?i)call for papers|download|征稿|征文|pdf`, label) {
			details = append(details, location.Href(false))
		} else if location.Hostname() == base.Hostname() {
			rel, _ := element.Attr("rel")
			if slices.Contains(strings.Fields(rel), "next") || pattern(`(?i)^(?:view all calls for papers|calls? for papers|collections|collections and calls for papers|next page|下一页|征稿启事|征稿通知|征文通知)$`, label) {
				indexes = append(indexes, location.Href(false))
			}
		}
	}
	details = append(details, indexes[:min(len(indexes), 3)]...)
	seen := map[string]bool{}
	return slices.DeleteFunc(details, func(value string) bool { has := seen[value]; seen[value] = true; return has })
}
