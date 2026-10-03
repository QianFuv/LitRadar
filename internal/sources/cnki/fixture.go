package cnki

import (
	"context"
	"fmt"
	"maps"
	"slices"
	"strings"
	"sync"

	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

// Transport owns domestic requests, transient sessions and attempt accounting.
type Transport interface {
	ResetTransientState(context.Context) error
	ResolveJournal(context.Context, JournalLocator) (any, error)
	YearIssues(context.Context, any) ([]any, error)
	IssueArticles(context.Context, any, any, uint64) (IssueArticlePage, error)
	ArticleDetail(context.Context, string, *string) (any, error)
	Attempts() []scholarly.Attempt
	DrainAttempts() []scholarly.Attempt
}

// FixtureData stores independent upstream response bodies and explicit failure controls.
type FixtureData struct {
	JournalSearchHtml        string              `json:"journal_search_html"`
	JournalDetailHtml        string              `json:"journal_detail_html"`
	YearIssuesHtml           string              `json:"year_issues_html"`
	IssueArticlePages        map[string][]string `json:"issue_article_pages"`
	ArticleDetailHtml        map[string]string   `json:"article_detail_html"`
	ArticleDetailStatusCodes map[string]uint16   `json:"article_detail_status_codes"`
	FailEndpoint             *string             `json:"fail_endpoint"`
}

// FixtureTransport replays HTML without changing parser-error attempt accounting.
type FixtureTransport struct {
	mutex    sync.Mutex
	data     FixtureData
	attempts []scholarly.Attempt
}

// NewFixtureTransport takes an owned copy of all fixture state.
func NewFixtureTransport(data FixtureData) *FixtureTransport {
	data.IssueArticlePages = maps.Clone(data.IssueArticlePages)
	for key, pages := range data.IssueArticlePages {
		data.IssueArticlePages[key] = slices.Clone(pages)
	}
	data.ArticleDetailHtml = maps.Clone(data.ArticleDetailHtml)
	data.ArticleDetailStatusCodes = maps.Clone(data.ArticleDetailStatusCodes)
	if data.FailEndpoint != nil {
		endpoint := *data.FailEndpoint
		data.FailEndpoint = &endpoint
	}
	return &FixtureTransport{data: data, attempts: []scholarly.Attempt{}}
}

// Clone copies fixture data and captured attempts without sharing mutable state.
func (fixture *FixtureTransport) Clone() *FixtureTransport {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	copy := NewFixtureTransport(fixture.data)
	copy.attempts = copyAttempts(fixture.attempts)
	return copy
}

// ResetTransientState is intentionally inert for deterministic fixtures.
func (fixture *FixtureTransport) ResetTransientState(context.Context) error { return nil }
func fixtureUrl(endpoint string, key *string) string {
	switch endpoint {
	case "journal_detail":
		return NaviBase + "/knavi/detail"
	case "year_issues":
		return NaviBase + "/knavi/journals/yearList"
	case "issue_articles":
		if key != nil {
			return NaviBase + "/knavi/journals/papers?yearIssue=" + *key
		}
	case "article_detail":
		if key != nil {
			return KnsBase + "/kcms2/article/abstract?v=" + *key
		}
	}
	return NaviBase + "/knavi/" + endpoint
}
func (fixture *FixtureTransport) record(endpoint string, key *string, isSuccess bool, message *string) {
	status := uint16(200)
	if !isSuccess {
		status = 500
	}
	method := "POST"
	if endpoint == "journal_detail" || endpoint == "article_detail" {
		method = "GET"
	}
	fixture.attempts = append(fixture.attempts, scholarly.Attempt{Service: "cnki", Endpoint: endpoint, Method: method, Url: fixtureUrl(endpoint, key), StatusCode: &status, DidSucceed: isSuccess, Error: message})
}
func (fixture *FixtureTransport) forced(endpoint string, key *string) error {
	if fixture.data.FailEndpoint == nil || *fixture.data.FailEndpoint != endpoint {
		return nil
	}
	message := "domestic CNKI fixture failed for " + endpoint
	fixture.record(endpoint, key, false, &message)
	return &Error{Kind: "Parse", Message: message}
}

// ResolveJournal uses detail identities directly, as the original fixture transport does.
func (fixture *FixtureTransport) ResolveJournal(ctx context.Context, locator JournalLocator) (any, error) {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	if err := fixture.forced("journal_detail", nil); err != nil {
		return nil, err
	}
	if strings.TrimSpace(fixture.data.JournalDetailHtml) == "" {
		fixture.record("journal_detail", nil, true, nil)
		return nil, nil
	}
	detail, err := ParseJournalDetail(fixture.data.JournalDetailHtml)
	if err != nil {
		return nil, err
	}
	fixture.record("journal_detail", nil, true, nil)
	if JournalDetailMatches(detail, locator) {
		return detail, nil
	}
	return nil, nil
}

// YearIssues parses the configured tree before recording a successful attempt.
func (fixture *FixtureTransport) YearIssues(ctx context.Context, journal any) ([]any, error) {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	if err := fixture.forced("year_issues", nil); err != nil {
		return nil, err
	}
	issues, err := ParseYearIssues(fixture.data.YearIssuesHtml)
	if err != nil {
		return nil, err
	}
	fixture.record("year_issues", nil, true, nil)
	return issues, nil
}
func jsonText(value any) *string {
	text, ok := value.(string)
	if !ok {
		return nil
	}
	return &text
}

// IssueArticles preserves key selection, missing-fixture errors and exact page identity.
func (fixture *FixtureTransport) IssueArticles(ctx context.Context, journal, issue any, pageIndex uint64) (IssueArticlePage, error) {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	key := firstValue(jsonText(field(issue, "year_issue_id")), jsonText(field(issue, "year_issue")))
	if key == nil {
		return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic CNKI issue missing year_issue_id"}
	}
	if err := fixture.forced("issue_articles", key); err != nil {
		return IssueArticlePage{}, err
	}
	pages := fixture.data.IssueArticlePages[*key]
	if pageIndex >= uint64(len(pages)) {
		return IssueArticlePage{}, &Error{Kind: "MissingFixture", Message: fmt.Sprintf("domestic CNKI fixture missing issue_articles page %d for %s", pageIndex, *key)}
	}
	page, err := ParseIssueArticles(pages[pageIndex], issue, pageIndex)
	if err != nil {
		return IssueArticlePage{}, err
	}
	attemptKey := fmt.Sprintf("%s:%d", *key, pageIndex)
	fixture.record("issue_articles", &attemptKey, true, nil)
	return page, nil
}

// ArticleDetail applies explicit status overrides before forced errors or missing content.
func (fixture *FixtureTransport) ArticleDetail(ctx context.Context, url string, platformId *string) (any, error) {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	key := url
	if platformId != nil {
		key = *platformId
	}
	if status, exists := fixture.data.ArticleDetailStatusCodes[key]; exists {
		message := "HTTP status"
		fixture.attempts = append(fixture.attempts, scholarly.Attempt{Service: "cnki", Endpoint: "article_detail", Method: "GET", Url: fixtureUrl("article_detail", &key), StatusCode: &status, Error: &message})
		return nil, httpStatusError(status)
	}
	if err := fixture.forced("article_detail", &key); err != nil {
		return nil, err
	}
	text, exists := fixture.data.ArticleDetailHtml[key]
	if !exists {
		return nil, &Error{Kind: "MissingFixture", Message: "domestic CNKI fixture missing article_detail for " + key}
	}
	detail, err := ParseArticleDetail(text, url)
	if err != nil {
		return nil, err
	}
	fixture.record("article_detail", &key, true, nil)
	return detail, nil
}
func httpStatusError(status uint16) error {
	return &Error{Kind: "Request", Message: fmt.Sprintf("domestic CNKI HTTP status %d", status)}
}
func copyAttempts(attempts []scholarly.Attempt) []scholarly.Attempt {
	result := append([]scholarly.Attempt{}, attempts...)
	for index, attempt := range result {
		if attempt.StatusCode != nil {
			status := *attempt.StatusCode
			result[index].StatusCode = &status
		}
		if attempt.Error != nil {
			message := *attempt.Error
			result[index].Error = &message
		}
	}
	return result
}

// Attempts returns an owned snapshot of captured requests.
func (fixture *FixtureTransport) Attempts() []scholarly.Attempt {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	return copyAttempts(fixture.attempts)
}

// DrainAttempts transfers captured attempts and leaves an empty buffer.
func (fixture *FixtureTransport) DrainAttempts() []scholarly.Attempt {
	fixture.mutex.Lock()
	defer fixture.mutex.Unlock()
	attempts := fixture.attempts
	fixture.attempts = []scholarly.Attempt{}
	return attempts
}
func jsonStrings(value any, scalar, array string) []string {
	result := []string{}
	if text := jsonText(field(value, scalar)); text != nil {
		result = append(result, *text)
	}
	if items, ok := field(value, array).([]any); ok {
		for _, item := range items {
			if text := jsonText(item); text != nil {
				result = append(result, *text)
			}
		}
	}
	return result
}

// JournalDetailMatches prioritizes validated ISSN identity whenever both sides provide it.
func JournalDetailMatches(detail any, locator JournalLocator) bool {
	issns := jsonStrings(detail, "issn", "issns")
	if eissn := jsonText(field(detail, "eissn")); eissn != nil {
		issns = append(issns, *eissn)
	}
	observed := NewJournalLocator(jsonStrings(detail, "title", "title_aliases"), issns)
	first, second := locator.normalizedTitles, observed.normalizedTitles
	if len(locator.normalizedIssns) > 0 && len(observed.normalizedIssns) > 0 {
		first, second = locator.normalizedIssns, observed.normalizedIssns
	}
	for key := range first {
		if second[key] {
			return true
		}
	}
	return false
}
