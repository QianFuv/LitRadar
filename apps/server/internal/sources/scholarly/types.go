// Package scholarly retrieves metadata from Crossref, OpenAlex and Semantic Scholar.
package scholarly

import (
	"context"
	"fmt"
)

const (
	Crossref              = "crossref"
	OpenAlex              = "openalex"
	SemanticScholar       = "semantic_scholar"
	SemanticScholarFields = "externalIds,title,isOpenAccess,abstract"
	OpenAlexDoiFields     = "doi,display_name,title,abstract_inverted_index,best_oa_location"
)

// Attempt captures one upstream attempt without exposing request credentials.
type Attempt struct {
	Service    string  `json:"service"`
	Endpoint   string  `json:"endpoint"`
	Method     string  `json:"method"`
	Url        string  `json:"url"`
	StatusCode *uint16 `json:"status_code"`
	DidSucceed bool    `json:"did_succeed"`
	DidRetry   bool    `json:"did_retry"`
	Error      *string `json:"error"`
}

// Error retains classification and a safe display message separately from upstream data.
type Error struct {
	Kind              string
	Service, Endpoint string
	StatusCode        uint16
	Body              any
	Message           string
}

func (failure *Error) Error() string {
	switch failure.Kind {
	case "HttpStatus":
		return fmt.Sprintf("%s %s failed with HTTP %d", failure.Service, failure.Endpoint, failure.StatusCode)
	case "Request":
		return fmt.Sprintf("%s %s request failed: %s", failure.Service, failure.Endpoint, failure.Message)
	default:
		return failure.Message
	}
}

// CrossrefQuery freezes either earliest discovery or inclusive creation/update bounds.
type CrossrefQuery struct {
	IsEarliest                bool
	CreatedFrom, CreatedUntil int64
	UpdatedFrom               *string
	UpdatedUntil              *int64
	Cursor                    *string
}

// CrossrefPage keeps the upstream count required to detect incomplete collections.
type CrossrefPage struct {
	Items        []any   `json:"items"`
	TotalResults uint64  `json:"total_results"`
	NextCursor   *string `json:"next_cursor"`
}

// WorksPage records when a rejected OpenAlex date filter restarts full pagination.
type WorksPage struct {
	Items                   []any   `json:"items"`
	NextCursor              *string `json:"next_cursor"`
	DidFallbackToUnfiltered bool    `json:"did_fallback_to_unfiltered"`
}

// Request carries a logical operation; Url is the frozen attempt URL, not an authority override.
type Request struct {
	Service, Endpoint, Method, Url string
	Issn, Title, SourceId          string
	Query                          CrossrefQuery
	FromSyncDate, Cursor           *string
	Dois                           []string
}

// Transport owns request execution, request batching and attempt accounting.
type Transport interface {
	Request(context.Context, Request) (any, error)
	PrepareOpenAlexDoiBatches([]string, int) ([][]string, error)
	RequestOpenAlexDoiBatches(context.Context, [][]string) ([]any, error)
	Attempts() []Attempt
	DrainAttempts() []Attempt
}
