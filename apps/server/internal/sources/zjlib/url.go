package zjlib

import (
	"net/url"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

type endpointFamily int

const (
	wwwFamily endpointFamily = iota
	shareFamily
	loginFamily
	proxyFamily
)

type endpoints struct {
	bases          [4]string
	entry, referer string
}

func defaultEndpoints() endpoints {
	return endpoints{bases: [4]string{WwwBase, ShareBase, LoginBase, ProxyBase}, entry: ShareBase + "/entry/area/35594/2120", referer: "http://10.18.17.173/kns55/"}
}
func hasOrigin(location, base *whatwg.Url) bool {
	return location.Username() == "" && location.Password() == "" && location.Scheme() == base.Scheme() && location.Hostname() == base.Hostname() && location.Port() == base.Port()
}
func safeEndpointPath(path string) bool {
	lower := asciiLower(path)
	return !strings.ContainsAny(path, `\`) && !strings.Contains(lower, "%2f") && !strings.Contains(lower, "%5c") && !strings.Contains(lower, "%2e")
}
func (allowed endpoints) family(location *whatwg.Url) (endpointFamily, error) {
	if location.Href(false) != location.Href(true) || !safeEndpointPath(location.Pathname()) {
		return 0, &Error{Kind: "Parse", Message: "ZJLib request used an unexpected endpoint."}
	}
	for index, value := range allowed.bases {
		base, err := whatwg.NewParser().Parse(value)
		if err != nil {
			continue
		}
		path := strings.TrimRight(base.Pathname(), "/")
		if hasOrigin(location, base) && (path == "" || location.Pathname() == path || strings.HasPrefix(location.Pathname(), path+"/")) {
			return endpointFamily(index), nil
		}
	}
	return 0, &Error{Kind: "Parse", Message: "ZJLib request used an unexpected endpoint."}
}
func (allowed endpoints) validate(location *whatwg.Url, expected endpointFamily) error {
	family, err := allowed.family(location)
	if err != nil {
		return err
	}
	if family != expected {
		return &Error{Kind: "Parse", Message: "ZJLib request used an unexpected endpoint family."}
	}
	return nil
}
func (allowed endpoints) parse(value string, expected endpointFamily) (*whatwg.Url, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "ZJLib request URL was invalid."}
	}
	if err = allowed.validate(location, expected); err != nil {
		return nil, err
	}
	return location, nil
}
func (allowed endpoints) join(base, reference string, expected endpointFamily) (string, error) {
	location, err := allowed.parse(base, expected)
	if err != nil {
		return "", err
	}
	joined, err := whatwg.NewParser().ParseRef(location.Href(false), reference)
	if err != nil {
		return "", &Error{Kind: "Parse", Message: "ZJLib response URL was invalid."}
	}
	if err = allowed.validate(joined, expected); err != nil {
		return "", err
	}
	return joined.Href(false), nil
}
func queryDict(value string) (map[string]string, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "URL query is invalid."}
	}
	query, err := url.ParseQuery(strings.TrimPrefix(location.Search(), "?"))
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "URL query contains invalid encoding."}
	}
	result := make(map[string]string, len(query))
	for name, values := range query {
		result[transport.LossyUtf8([]byte(name))] = transport.LossyUtf8([]byte(values[len(values)-1]))
	}
	return result, nil
}
func titleFromPdfURL(value string) *string {
	query, err := queryDict(value)
	if err != nil {
		return nil
	}
	if title, ok := query["filetitle"]; ok {
		return cleanText(title)
	}
	if title, ok := query["filename"]; ok {
		return cleanText(title)
	}
	return nil
}

// ParseSearchResults preserves result order, canonical deduplication and row-level PDF hints.
func ParseSearchResults(text, base string) ([]SearchResult, error) {
	return parseSearchResults(text, base, defaultEndpoints())
}

// parseSearchResults admits and deduplicates canonical detail URLs before row hints.
func parseSearchResults(text, base string, allowed endpoints) ([]SearchResult, error) {
	lower := asciiLower(text)
	seen := map[string]bool{}
	results := []SearchResult{}
	for _, anchor := range anchorLinks(text) {
		if !strings.Contains(asciiLower(anchor.href), "/kns55/detail/detail.aspx") {
			continue
		}
		detail, err := allowed.join(base, decodeHTML(anchor.href), proxyFamily)
		if err != nil {
			return nil, err
		}
		if seen[detail] {
			continue
		}
		seen[detail] = true
		row := searchResultRow(text, lower, anchor)
		result, err := buildSearchResult(anchor, detail, row, base, allowed, len(results)+1)
		if err != nil {
			return nil, err
		}
		results = append(results, result)
	}
	return results, nil
}
func extractDownloadURL(text, base string, allowed endpoints, requirePdf bool) (*string, error) {
	for _, anchor := range anchorLinks(text) {
		href := anchor.href
		if requirePdf {
			href = decodeHTML(href)
		}
		lower := asciiLower(href)
		if !strings.Contains(lower, "download.aspx") {
			continue
		}
		if requirePdf && !strings.Contains(lower, "dflag=pdfdown") && !strings.Contains(asciiLower(stripTags(anchor.body)), "pdf") {
			continue
		}
		if !requirePdf {
			href = decodeHTML(href)
		}
		value, err := allowed.join(base, href, proxyFamily)
		if err != nil {
			return nil, err
		}
		return &value, nil
	}
	return nil, nil
}

// searchResultRow keeps the original enclosing-row and missing-tag bounds.
func searchResultRow(text, lower string, anchor anchorLink) string {
	start := strings.LastIndex(lower[:anchor.start], "<tr")
	if start < 0 {
		start = anchor.start
	}
	end := strings.Index(lower[anchor.end:], "</tr>")
	if end < 0 {
		end = anchor.end
	} else {
		end += anchor.end + 5
	}
	return text[start:end]
}

// buildSearchResult resolves title before row hints and retains case-sensitive query metadata.
func buildSearchResult(anchor anchorLink, detail, row, base string, allowed endpoints, index int) (SearchResult, error) {
	query, err := queryDict(detail)
	if err != nil {
		return SearchResult{}, err
	}
	title := anchorTitle(anchor.body)
	if title == nil {
		if value, ok := query["FileName"]; ok {
			title = &value
		} else {
			title = new("result-" + strconv.Itoa(index))
		}
	}
	download, err := extractDownloadURL(row, base, allowed, false)
	if err != nil {
		return SearchResult{}, err
	}
	result := SearchResult{Index: uint64(index), Title: *title, DetailURL: detail, DownloadURL: download}
	if value, ok := query["FileName"]; ok {
		result.FileName = &value
	}
	if value, ok := query["DbName"]; ok {
		result.DBName = &value
	}
	if value, ok := query["DbCode"]; ok {
		result.DBCode = &value
	}
	return result, nil
}
