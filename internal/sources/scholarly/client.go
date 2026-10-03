package scholarly

import (
	"context"
	"log/slog"
	"sync"
)

// Client serializes operations that share pagination and plan-restriction state.
type Client struct {
	mutex                          sync.Mutex
	transport                      Transport
	hasSemanticScholarKey          bool
	isCreatedDateFilterUnavailable bool
}

// NewClient binds transport state and the configured enrichment capability.
func NewClient(transport Transport, hasSemanticScholarKey bool) *Client {
	return &Client{transport: transport, hasSemanticScholarKey: hasSemanticScholarKey}
}

// FetchCrossrefPage requires a precise unsigned count before accepting a works array.
func (client *Client) FetchCrossrefPage(ctx context.Context, issn string, query CrossrefQuery) (CrossrefPage, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	payload, err := client.transport.Request(ctx, Request{Service: Crossref, Endpoint: "journal_works", Method: "GET", Url: "https://api.crossref.org/journals/" + issn + "/works", Issn: issn, Query: query})
	if err != nil {
		return CrossrefPage{}, err
	}
	message := field(payload, "message")
	var total uint64
	var hasTotal bool
	if parsed, ok := sourceNumber(field(message, "total-results")); ok {
		total, hasTotal = parsed.AsUint64()
	}
	if !hasTotal {
		return CrossrefPage{}, &Error{Kind: "InvalidFixture", Message: "Crossref response has no valid total-results"}
	}
	items, ok := field(message, "items").([]any)
	if !ok {
		return CrossrefPage{}, &Error{Kind: "InvalidFixture", Message: "Crossref response has no items array"}
	}
	return CrossrefPage{Items: items, TotalResults: total, NextCursor: optionalString(field(message, "next-cursor"))}, nil
}

// FetchOpenAlexSourceByIssns checks every supplied candidate until identity matches.
func (client *Client) FetchOpenAlexSourceByIssns(ctx context.Context, issns []string) (any, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	for _, issn := range issns {
		payload, err := client.transport.Request(ctx, Request{Service: OpenAlex, Endpoint: "sources", Method: "GET", Url: "https://api.openalex.org/sources?filter=issn:" + issn + "&api_key=SECRET", Issn: issn})
		if err != nil {
			return nil, err
		}
		for _, item := range array(field(payload, "results")) {
			if sourceMatchesIssn(item, issn) {
				return item, nil
			}
		}
	}
	return nil, nil
}

// FetchOpenAlexSourceByTitle accepts only a normalized exact title match.
func (client *Client) FetchOpenAlexSourceByTitle(ctx context.Context, title string) (any, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	normalized := normalizedTitle(title)
	if normalized == "" {
		return nil, nil
	}
	payload, err := client.transport.Request(ctx, Request{Service: OpenAlex, Endpoint: "source_search", Method: "GET", Url: "https://api.openalex.org/sources?search=" + title + "&api_key=SECRET", Title: title})
	if err != nil {
		return nil, err
	}
	for _, item := range array(field(payload, "results")) {
		if name, ok := field(item, "display_name").(string); ok && normalizedTitle(name) == normalized {
			return item, nil
		}
	}
	return nil, nil
}

func sourceWorksRequest(sourceId string, fromDate, cursor *string) Request {
	return Request{Service: OpenAlex, Endpoint: "source_works", Method: "GET", Url: "https://api.openalex.org/works?filter=primary_location.source.id:" + sourceId + "&api_key=SECRET", SourceId: sourceId, FromSyncDate: fromDate, Cursor: cursor}
}

// FetchOpenAlexWorksBySourcePage remembers paid-filter rejection across operations.
func (client *Client) FetchOpenAlexWorksBySourcePage(ctx context.Context, sourceId string, fromDate, cursor *string) (WorksPage, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	effectiveDate := fromDate
	if client.isCreatedDateFilterUnavailable {
		effectiveDate = nil
	}
	hasFallback := fromDate != nil && effectiveDate == nil
	if hasFallback {
		cursor = nil
	}
	payload, err := client.transport.Request(ctx, sourceWorksRequest(sourceId, effectiveDate, cursor))
	if err != nil && effectiveDate != nil && isPlanRestriction(err) {
		slog.WarnContext(ctx, "source fallback activated", "event", "source.fallback.activated", "component", "source", "provider", OpenAlex, "endpoint", "source_works", "reason", "plan_restriction", "fallback", "full_source_pages")
		client.isCreatedDateFilterUnavailable = true
		hasFallback = true
		payload, err = client.transport.Request(ctx, sourceWorksRequest(sourceId, nil, nil))
	}
	if err != nil {
		return WorksPage{}, err
	}
	return WorksPage{Items: array(field(payload, "results")), NextCursor: optionalString(field(field(payload, "meta"), "next_cursor")), DidFallbackToUnfiltered: hasFallback}, nil
}

// FetchOpenAlexByDois partitions normalized IDs and preserves last-returned payloads.
func (client *Client) FetchOpenAlexByDois(ctx context.Context, dois []string, batchSize int) (map[string]any, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	batches, err := client.transport.PrepareOpenAlexDoiBatches(uniqueDois(dois), batchSize)
	if err != nil {
		return nil, err
	}
	payloads, err := client.transport.RequestOpenAlexDoiBatches(ctx, batches)
	if err != nil {
		return nil, err
	}
	if len(payloads) != len(batches) {
		return nil, &Error{Kind: "InvalidFixture", Message: "OpenAlex DOI batch response count does not match the request count."}
	}
	results := map[string]any{}
	for _, payload := range payloads {
		for _, item := range array(field(payload, "results")) {
			if doi := NormalizeDoi(field(item, "doi")); doi != nil {
				results[*doi] = item
			}
		}
	}
	return results, nil
}

// FetchSemanticScholarByDois tolerates only the upstream empty-ID batch error.
func (client *Client) FetchSemanticScholarByDois(ctx context.Context, dois []string, batchSize int) (map[string]any, error) {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	normalized := uniqueDois(dois)
	results := map[string]any{}
	if len(normalized) == 0 {
		return results, nil
	}
	if !client.hasSemanticScholarKey {
		return nil, &Error{Kind: "Configuration", Message: "Semantic Scholar API key is required for DOI enrichment."}
	}
	batchSize = max(1, min(batchSize, 500))
	for offset := 0; offset < len(normalized); offset += batchSize {
		batch := append([]string{}, normalized[offset:min(offset+batchSize, len(normalized))]...)
		payload, err := client.transport.Request(ctx, Request{Service: SemanticScholar, Endpoint: "paper_batch", Method: "POST", Url: "https://api.semanticscholar.org/graph/v1/paper/batch?fields=" + SemanticScholarFields + "&x-api-key=SECRET", Dois: batch})
		if err != nil {
			if isNoValidIds(err) {
				continue
			}
			return nil, err
		}
		for _, item := range array(payload) {
			if doi := NormalizeDoi(field(field(item, "externalIds"), "DOI")); doi != nil {
				results[*doi] = item
			}
		}
	}
	return results, nil
}

// Attempts returns an isolated snapshot of captured request attempts.
func (client *Client) Attempts() []Attempt {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	return client.transport.Attempts()
}

// DrainAttempts clears attempt accounting while retaining source state.
func (client *Client) DrainAttempts() []Attempt {
	client.mutex.Lock()
	defer client.mutex.Unlock()
	return client.transport.DrainAttempts()
}
