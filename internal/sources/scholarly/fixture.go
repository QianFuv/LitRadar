package scholarly

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"sync"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// FixtureData supplies deterministic upstream responses for offline indexing.
type FixtureData struct {
	CrossrefStatus                             *uint16        `json:"crossref_status"`
	CrossrefWorks                              []any          `json:"crossref_works"`
	CrossrefWorkPages                          [][]any        `json:"crossref_work_pages"`
	OpenAlexSourceByIssns                      any            `json:"openalex_source_by_issns"`
	OpenAlexSourceByTitle                      any            `json:"openalex_source_by_title"`
	OpenAlexSourceWorks                        []any          `json:"openalex_source_works"`
	OpenAlexSourceWorkPages                    [][]any        `json:"openalex_source_work_pages"`
	IsOpenAlexSourceWorksPlanRestricted        bool           `json:"openalex_source_works_plan_restricted"`
	OpenAlexSourceWorksPlanRestrictedAfterPage *uint64        `json:"openalex_source_works_plan_restricted_after_page"`
	OpenAlexSourceWorksStatus                  *uint16        `json:"openalex_source_works_status"`
	OpenAlexByDoi                              map[string]any `json:"openalex_by_doi"`
	SemanticScholarStatus                      *uint16        `json:"semantic_scholar_status"`
	SemanticScholarError                       *string        `json:"semantic_scholar_error"`
	SemanticScholarByDoi                       map[string]any `json:"semantic_scholar_by_doi"`
}

// Captures retains source-specific request history independently of attempt draining.
type Captures struct {
	SemanticScholarBatches [][]string `json:"semantic_scholar_batches"`
	OpenAlexDoiBatches     [][]string `json:"openalex_doi_batches"`
	SourceLookupIssns      []string   `json:"source_lookup_issns"`
	SourceLookupTitles     []string   `json:"source_lookup_titles"`
	JournalWorkRequests    [][2]any   `json:"journal_work_requests"`
	SourceWorkRequests     [][2]any   `json:"source_work_requests"`
}

// FixtureTransport preserves the original fixture's shared Crossref pagination offset.
type FixtureTransport struct {
	mutex             sync.Mutex
	data              FixtureData
	attempts          []Attempt
	captures          Captures
	crossrefPageIndex int
}

// NewFixtureTransport takes an isolated copy of fixture payloads.
func NewFixtureTransport(data FixtureData) *FixtureTransport {
	data.CrossrefWorks = cloneArray(data.CrossrefWorks)
	data.CrossrefWorkPages = clonePages(data.CrossrefWorkPages)
	data.OpenAlexSourceByIssns = cloneValue(data.OpenAlexSourceByIssns)
	data.OpenAlexSourceByTitle = cloneValue(data.OpenAlexSourceByTitle)
	data.OpenAlexSourceWorks = cloneArray(data.OpenAlexSourceWorks)
	data.OpenAlexSourceWorkPages = clonePages(data.OpenAlexSourceWorkPages)
	data.OpenAlexByDoi = cloneMap(data.OpenAlexByDoi)
	data.SemanticScholarByDoi = cloneMap(data.SemanticScholarByDoi)
	data.CrossrefStatus = clonePointer(data.CrossrefStatus)
	data.OpenAlexSourceWorksStatus = clonePointer(data.OpenAlexSourceWorksStatus)
	data.SemanticScholarStatus = clonePointer(data.SemanticScholarStatus)
	data.SemanticScholarError = clonePointer(data.SemanticScholarError)
	data.OpenAlexSourceWorksPlanRestrictedAfterPage = clonePointer(data.OpenAlexSourceWorksPlanRestrictedAfterPage)
	return &FixtureTransport{data: data, attempts: []Attempt{}, captures: Captures{SemanticScholarBatches: [][]string{}, OpenAlexDoiBatches: [][]string{}, SourceLookupIssns: []string{}, SourceLookupTitles: []string{}, JournalWorkRequests: [][2]any{}, SourceWorkRequests: [][2]any{}}}
}

// Request executes one deterministic logical request without network access.
func (transport *FixtureTransport) Request(ctx context.Context, request Request) (any, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	transport.mutex.Lock()
	defer transport.mutex.Unlock()
	switch request.Service + "/" + request.Endpoint {
	case Crossref + "/journal_works":
		var date any
		if !request.Query.IsEarliest && request.Query.UpdatedFrom != nil {
			date = *request.Query.UpdatedFrom
		}
		transport.captures.JournalWorkRequests = append(transport.captures.JournalWorkRequests, [2]any{request.Issn, date})
		if status := fixtureStatus(transport.data.CrossrefStatus); status != 200 {
			return nil, transport.httpError(request, status, map[string]any{"message": "fixture crossref failure"})
		}
		transport.record(request, 200, nil)
		works := transport.data.CrossrefWorks
		if len(transport.data.CrossrefWorkPages) > 0 {
			works = []any{}
			for _, page := range transport.data.CrossrefWorkPages {
				works = append(works, page...)
			}
		}
		matching := []any{}
		for _, work := range works {
			created := fixtureCreated(work)
			if created <= request.Query.CreatedUntil && (request.Query.IsEarliest || created >= request.Query.CreatedFrom) {
				matching = append(matching, work)
			}
		}
		offset, rows := 0, 1
		if request.Query.IsEarliest {
			sort.SliceStable(matching, func(first, second int) bool {
				return fixtureCreated(matching[first]) < fixtureCreated(matching[second])
			})
		} else {
			if request.Query.Cursor == nil || *request.Query.Cursor == "*" {
				transport.crossrefPageIndex = 0
			}
			offset, rows = transport.crossrefPageIndex, 225
		}
		items := []any{}
		for index := offset; index < min(offset+rows, len(matching)); index++ {
			work := cloneValue(matching[index])
			if object, ok := work.(map[string]any); ok {
				if _, hasCreated := object["created"]; !hasCreated {
					object["created"] = map[string]any{"timestamp": json.Number("0")}
				}
			}
			items = append(items, work)
		}
		transport.crossrefPageIndex = offset + len(items)
		var cursor any
		if !request.Query.IsEarliest && request.Query.Cursor != nil && len(items) > 0 {
			cursor = "stateful-crossref-cursor"
		}
		return map[string]any{"message": map[string]any{"items": items, "total-results": uint64(len(matching)), "next-cursor": cursor}}, nil
	case OpenAlex + "/sources", OpenAlex + "/source_search":
		value := transport.data.OpenAlexSourceByIssns
		if request.Endpoint == "sources" {
			transport.captures.SourceLookupIssns = append(transport.captures.SourceLookupIssns, request.Issn)
		} else {
			value = transport.data.OpenAlexSourceByTitle
			transport.captures.SourceLookupTitles = append(transport.captures.SourceLookupTitles, request.Title)
		}
		transport.record(request, 200, nil)
		items := []any{}
		if value != nil {
			items = append(items, cloneValue(value))
		}
		return map[string]any{"results": items}, nil
	case OpenAlex + "/source_works":
		var date any
		if request.FromSyncDate != nil {
			date = *request.FromSyncDate
		}
		transport.captures.SourceWorkRequests = append(transport.captures.SourceWorkRequests, [2]any{request.SourceId, date})
		pageIndex := fixturePageIndex(request.Cursor)
		threshold := transport.data.OpenAlexSourceWorksPlanRestrictedAfterPage
		if request.FromSyncDate != nil && (transport.data.IsOpenAlexSourceWorksPlanRestricted || threshold != nil && pageIndex >= *threshold) {
			return nil, transport.httpError(request, 429, map[string]any{"error": "Plan upgrade required", "message": "The from_created_date filter requires a Premium plan."})
		}
		if status := fixtureStatus(transport.data.OpenAlexSourceWorksStatus); status != 200 {
			return nil, transport.httpError(request, status, map[string]any{"error": "fixture OpenAlex source works failure"})
		}
		transport.record(request, 200, nil)
		items := []any{}
		var cursor any
		pages := transport.data.OpenAlexSourceWorkPages
		if len(pages) == 0 {
			if pageIndex == 0 {
				items = cloneArray(transport.data.OpenAlexSourceWorks)
			}
		} else if pageIndex < uint64(len(pages)) {
			items = cloneArray(pages[pageIndex])
			if pageIndex < uint64(len(pages)-1) {
				cursor = "fixture-page-" + strconv.FormatUint(pageIndex+1, 10)
			}
		}
		return map[string]any{"results": items, "meta": map[string]any{"next_cursor": cursor}}, nil
	case OpenAlex + "/works":
		transport.captures.OpenAlexDoiBatches = append(transport.captures.OpenAlexDoiBatches, append([]string{}, request.Dois...))
		transport.record(request, 200, nil)
		return map[string]any{"results": fixtureDoiItems(transport.data.OpenAlexByDoi, request.Dois)}, nil
	case SemanticScholar + "/paper_batch":
		transport.captures.SemanticScholarBatches = append(transport.captures.SemanticScholarBatches, append([]string{}, request.Dois...))
		if status := fixtureStatus(transport.data.SemanticScholarStatus); status != 200 {
			message := "fixture semantic scholar failure"
			if transport.data.SemanticScholarError != nil {
				message = *transport.data.SemanticScholarError
			}
			return nil, transport.httpError(request, status, map[string]any{"error": message})
		}
		transport.record(request, 200, nil)
		return fixtureDoiItems(transport.data.SemanticScholarByDoi, request.Dois), nil
	default:
		return nil, &Error{Kind: "InvalidFixture", Message: "unsupported scholarly fixture request"}
	}
}

// PrepareOpenAlexDoiBatches applies the fixture's credential-free URL budget.
func (transport *FixtureTransport) PrepareOpenAlexDoiBatches(dois []string, batchSize int) ([][]string, error) {
	return PartitionOpenAlexDois(dois, batchSize, nil)
}

// RequestOpenAlexDoiBatches preserves batch order and stops at the first failure.
func (transport *FixtureTransport) RequestOpenAlexDoiBatches(ctx context.Context, batches [][]string) ([]any, error) {
	results := []any{}
	for _, batch := range batches {
		payload, err := transport.Request(ctx, Request{Service: OpenAlex, Endpoint: "works", Method: "GET", Url: "https://api.openalex.org/works?filter=REDACTED&api_key=SECRET", Dois: batch})
		if err != nil {
			return nil, err
		}
		results = append(results, payload)
	}
	return results, nil
}

// Attempts returns copied attempts so callers cannot mutate transport history.
func (transport *FixtureTransport) Attempts() []Attempt {
	transport.mutex.Lock()
	defer transport.mutex.Unlock()
	return cloneAttempts(transport.attempts)
}

// DrainAttempts clears only the accounting buffer.
func (transport *FixtureTransport) DrainAttempts() []Attempt {
	transport.mutex.Lock()
	defer transport.mutex.Unlock()
	result := cloneAttempts(transport.attempts)
	transport.attempts = []Attempt{}
	return result
}

// Captures returns independent request histories.
func (transport *FixtureTransport) Captures() Captures {
	transport.mutex.Lock()
	defer transport.mutex.Unlock()
	result := transport.captures
	result.SemanticScholarBatches = cloneBatches(result.SemanticScholarBatches)
	result.OpenAlexDoiBatches = cloneBatches(result.OpenAlexDoiBatches)
	result.SourceLookupIssns = append([]string{}, result.SourceLookupIssns...)
	result.SourceLookupTitles = append([]string{}, result.SourceLookupTitles...)
	result.JournalWorkRequests = append([][2]any{}, result.JournalWorkRequests...)
	result.SourceWorkRequests = append([][2]any{}, result.SourceWorkRequests...)
	return result
}

func (transport *FixtureTransport) record(request Request, status uint16, failure *string) {
	transport.attempts = append(transport.attempts, Attempt{Service: request.Service, Endpoint: request.Endpoint, Method: request.Method, Url: request.Url, StatusCode: &status, DidSucceed: failure == nil, Error: failure})
}
func (transport *FixtureTransport) httpError(request Request, status uint16, body any) error {
	encoded, _ := domain.Json(body)
	message := fmt.Sprintf("HTTP %d: %s", status, encoded)
	transport.record(request, status, &message)
	return &Error{Kind: "HttpStatus", Service: request.Service, Endpoint: request.Endpoint, StatusCode: status, Body: body}
}
func fixtureStatus(status *uint16) uint16 {
	if status == nil {
		return 200
	}
	return *status
}
func fixtureCreated(value any) int64 {
	var milliseconds int64
	if parsed, ok := sourceNumber(field(field(value, "created"), "timestamp")); ok {
		milliseconds, _ = parsed.AsInt64()
	}
	seconds := milliseconds / 1000
	if milliseconds%1000 < 0 {
		seconds--
	}
	return seconds
}
func fixturePageIndex(cursor *string) uint64 {
	if cursor == nil || !strings.HasPrefix(*cursor, "fixture-page-") {
		return 0
	}
	text := strings.TrimPrefix(*cursor, "fixture-page-")
	text = strings.TrimPrefix(text, "+")
	value, err := strconv.ParseUint(text, 10, 64)
	if err != nil {
		return 0
	}
	return value
}
func fixtureDoiItems(values map[string]any, dois []string) []any {
	items := []any{}
	for _, doi := range dois {
		if value, ok := values[doi]; ok {
			items = append(items, cloneValue(value))
		}
	}
	return items
}
func cloneValue(value any) any {
	switch item := value.(type) {
	case []any:
		return cloneArray(item)
	case map[string]any:
		return cloneMap(item)
	default:
		return value
	}
}
func cloneMap(value map[string]any) map[string]any {
	result := make(map[string]any, len(value))
	for key, item := range value {
		result[key] = cloneValue(item)
	}
	return result
}
func cloneArray(value []any) []any {
	result := make([]any, len(value))
	for index, item := range value {
		result[index] = cloneValue(item)
	}
	return result
}
func clonePages(value [][]any) [][]any {
	result := make([][]any, len(value))
	for index, item := range value {
		result[index] = cloneArray(item)
	}
	return result
}
func cloneBatches(value [][]string) [][]string {
	result := make([][]string, len(value))
	for index, item := range value {
		result[index] = append([]string{}, item...)
	}
	return result
}
func clonePointer[Value any](value *Value) *Value {
	if value == nil {
		return nil
	}
	result := *value
	return &result
}
func cloneAttempts(values []Attempt) []Attempt {
	result := append([]Attempt{}, values...)
	for index := range result {
		result[index].Error = clonePointer(result[index].Error)
		result[index].StatusCode = clonePointer(result[index].StatusCode)
	}
	return result
}
