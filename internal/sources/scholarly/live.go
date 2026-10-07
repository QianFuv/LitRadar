package scholarly

import (
	"context"
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

const openAlexSourceFields = "id,display_name,issn_l,issn,works_count"
const openAlexWorkFields = "id,doi,title,display_name,publication_year,publication_date,language,cited_by_count,is_retracted,primary_location,locations,open_access,best_oa_location,authorships,ids,biblio,abstract_inverted_index,topics,primary_topic,funders,awards"
const scholarlyMaximumBytes = 16 * 1024 * 1024

// LiveTransport owns sequential source state and shares OpenAlex admission with its clones.
type LiveTransport struct {
	mutex                                    *sync.Mutex
	client                                   *http.Client
	config                                   LiveConfig
	attempts                                 []Attempt
	crossref                                 crossrefSchedule
	semantic                                 *semanticScheduler
	openAlex                                 *sharedOpenAlex
	workerCount                              int
	deadline                                 time.Time
	crossrefBase, openAlexBase, semanticBase string
}

// NewLiveTransport constructs explicit networking and bounded enrichment workers.
func NewLiveTransport(config LiveConfig, workerCount int, proxy transport.Proxy) (*LiveTransport, error) {
	if workerCount < 1 || workerCount > 32 {
		return nil, &Error{Kind: "Configuration", Message: "scholarly worker_count must be between 1 and 32"}
	}
	wire, err := proxy.ClientTransport()
	if err != nil {
		return nil, &Error{Kind: "Configuration", Message: err.Error()}
	}
	config = cloneConfig(config)
	epoch := milliseconds(config.ScheduleEpochUnixMillis)
	state := newOpenAlexScheduler(len(config.OpenAlexApiKeys), config.SemanticScholarWorkerId, config.SemanticScholarProcessCount, epoch, saturatingMultiply(uint64(workerCount), max(config.SemanticScholarProcessCount, 1)))
	return &LiveTransport{mutex: &sync.Mutex{}, client: &http.Client{Transport: wire, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, config: config, attempts: []Attempt{}, crossref: newCrossrefSchedule(config), semantic: newSemanticScheduler(len(config.SemanticScholarApiKeys), config.SemanticScholarWorkerId, config.SemanticScholarProcessCount, epoch, milliseconds(max(config.SemanticScholarBaseIntervalMs, 1100))), openAlex: &sharedOpenAlex{state: state, changed: make(chan struct{})}, workerCount: workerCount, crossrefBase: "https://api.crossref.org/v1", openAlexBase: "https://api.openalex.org", semanticBase: "https://api.semanticscholar.org/graph/v1"}, nil
}

// Clone copies request histories and per-transport schedules while sharing OpenAlex quotas.
func (live *LiveTransport) Clone() *LiveTransport {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	semantic := *live.semantic
	semantic.Slots = append([]keyState{}, live.semantic.Slots...)
	for index := range semantic.Slots {
		semantic.Slots[index].Cooldown = clonePointer(semantic.Slots[index].Cooldown)
	}
	return &LiveTransport{mutex: &sync.Mutex{}, client: live.client, config: cloneConfig(live.config), attempts: cloneAttempts(live.attempts), crossref: live.crossref, semantic: &semantic, openAlex: live.openAlex, workerCount: live.workerCount, deadline: live.deadline, crossrefBase: live.crossrefBase, openAlexBase: live.openAlexBase, semanticBase: live.semanticBase}
}

// WithDeadline tightens the caller-owned ceiling on all subsequent operations.
func (live *LiveTransport) WithDeadline(deadline time.Time) *LiveTransport {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	if !deadline.IsZero() && (live.deadline.IsZero() || deadline.Before(live.deadline)) {
		live.deadline = deadline
	}
	return live
}

// CloseIdleConnections releases pooled sockets without changing provider state.
func (live *LiveTransport) CloseIdleConnections() { live.client.CloseIdleConnections() }
func (live LiveTransport) String() string         { return "LiveScholarlyTransport(credentials=[REDACTED])" }
func (live LiveTransport) GoString() string       { return live.String() }
func (live LiveTransport) LogValue() slog.Value   { return slog.StringValue(live.String()) }

// Request executes a typed operation against its fixed upstream service.
func (live *LiveTransport) Request(ctx context.Context, request Request) (any, error) {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	switch request.Service + "/" + request.Endpoint {
	case Crossref + "/journal_works":
		query, err := CrossrefParameters(request.Query)
		if err != nil {
			return nil, err
		}
		if len(live.config.CrossrefMailtos) > 0 {
			query = append(query, QueryPair{"mailto", live.config.CrossrefMailtos[0]})
		}
		return live.executeCrossref(ctx, live.crossrefBase+"/journals/"+request.Issn+"/works", query)
	case OpenAlex + "/sources":
		return live.openAlexRequest(ctx, "sources", live.openAlexBase+"/sources", []QueryPair{{"filter", "issn:" + request.Issn}, {"per-page", "5"}, {"select", openAlexSourceFields}})
	case OpenAlex + "/source_search":
		return live.openAlexRequest(ctx, "source_search", live.openAlexBase+"/sources", []QueryPair{{"search", request.Title}, {"per-page", "5"}, {"select", openAlexSourceFields}})
	case OpenAlex + "/source_works":
		return live.requestOpenAlexSourceWorks(ctx, request)
	case OpenAlex + "/works":
		return live.openAlexRequest(ctx, "works", live.openAlexBase+"/works", OpenAlexDoiQuery(request.Dois, nil))
	case SemanticScholar + "/paper_batch":
		ids := make([]any, len(request.Dois))
		for index, doi := range request.Dois {
			ids[index] = "DOI:" + doi
		}
		return live.executeSemantic(ctx, live.semanticBase+"/paper/batch", []QueryPair{{"fields", SemanticScholarFields}}, map[string]any{"ids": ids})
	default:
		return nil, &Error{Kind: "Configuration", Message: "unsupported scholarly source operation"}
	}
}

// PrepareOpenAlexDoiBatches reserves URL space for the longest configured credential.
func (live *LiveTransport) PrepareOpenAlexDoiBatches(dois []string, batchSize int) ([][]string, error) {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	var longest *string
	for _, key := range live.config.OpenAlexApiKeys {
		if longest == nil || len(key) >= len(*longest) {
			longest = &key
		}
	}
	return PartitionOpenAlexDois(dois, batchSize, longest)
}

// RequestOpenAlexDoiBatches stops new admissions on error and joins already admitted work.
func (live *LiveTransport) RequestOpenAlexDoiBatches(ctx context.Context, batches [][]string) ([]any, error) {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	if len(batches) == 0 {
		return []any{}, nil
	}
	var admission sync.Mutex
	next := 0
	shouldStop := false
	executions := make([]*sourceExecution, len(batches))
	var workers sync.WaitGroup
	var didPanic atomic.Bool
	for range min(live.workerCount, len(batches)) {
		workers.Go(func() {
			defer func() {
				if recover() != nil {
					didPanic.Store(true)
				}
			}()
			for {
				admission.Lock()
				if shouldStop || next == len(batches) {
					admission.Unlock()
					return
				}
				index := next
				next++
				admission.Unlock()
				execution := live.executeOpenAlex(ctx, "works", live.openAlexBase+"/works", OpenAlexDoiQuery(batches[index], nil))
				admission.Lock()
				executions[index] = &execution
				shouldStop = shouldStop || execution.err != nil
				admission.Unlock()
			}
		})
	}
	workers.Wait()
	if didPanic.Load() {
		return nil, &Error{Kind: "Request", Service: OpenAlex, Endpoint: "works", Message: "bounded OpenAlex worker failed"}
	}
	payloads := []any{}
	var firstError error
	for _, execution := range executions {
		if execution == nil {
			continue
		}
		live.recordExecutions(ctx, execution.attempts)
		if execution.err != nil {
			if firstError == nil {
				firstError = execution.err
			}
		} else {
			payloads = append(payloads, execution.payload)
		}
	}
	if firstError != nil {
		return nil, firstError
	}
	return payloads, nil
}

// Attempts returns a copied attempt snapshot.
func (live *LiveTransport) Attempts() []Attempt {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	return cloneAttempts(live.attempts)
}

// DrainAttempts clears only the captured attempt buffer.
func (live *LiveTransport) DrainAttempts() []Attempt {
	live.mutex.Lock()
	defer live.mutex.Unlock()
	result := cloneAttempts(live.attempts)
	live.attempts = []Attempt{}
	return result
}

func (live *LiveTransport) logicalDeadline(ctx context.Context) time.Time {
	deadline := live.deadline
	if caller, ok := ctx.Deadline(); ok && (deadline.IsZero() || caller.Before(deadline)) {
		deadline = caller
	}
	return transport.LogicalDeadline(deadline)
}

type executionAttempt struct {
	attempt         Attempt
	Number, KeySlot int
	WillRetry       bool
	ErrorKind       string
	DurationMillis  uint64
}
type sourceExecution struct {
	payload  any
	err      error
	attempts []executionAttempt
}

func (live *LiveTransport) recordExecutions(ctx context.Context, attempts []executionAttempt) {
	for _, record := range attempts {
		outcome := "failure"
		if record.attempt.DidSucceed {
			outcome = "success"
		}
		status := uint16(0)
		if record.attempt.StatusCode != nil {
			status = *record.attempt.StatusCode
		}
		slog.InfoContext(ctx, "source attempt", "event", "source."+record.attempt.Service+".attempt", "component", "source", "provider", record.attempt.Service, "endpoint", record.attempt.Endpoint, "method", record.attempt.Method, "attempt", record.Number, "key_slot", record.KeySlot, "outcome", outcome, "error_kind", record.ErrorKind, "http_status", status, "has_http_status", record.attempt.StatusCode != nil, "is_retry", record.attempt.DidRetry, "will_retry", record.WillRetry, "duration_ms", record.DurationMillis)
		live.attempts = append(live.attempts, record.attempt)
	}
}
func newAttempt(service, endpoint, method, url string, number, keySlot int, status *uint16, isSuccess, willRetry bool, errorKind string, started time.Time) executionAttempt {
	var message *string
	if !isSuccess {
		message = &errorKind
	}
	return executionAttempt{attempt: Attempt{Service: service, Endpoint: endpoint, Method: method, Url: RedactUrl(url), StatusCode: status, DidSucceed: isSuccess, DidRetry: number > 1, Error: message}, Number: number, KeySlot: keySlot, WillRetry: willRetry, ErrorKind: errorKind, DurationMillis: uint64(max(time.Since(started).Milliseconds(), 0))}
}
func (live *LiveTransport) openAlexRequest(ctx context.Context, endpoint, url string, query []QueryPair) (any, error) {
	execution := live.executeOpenAlex(ctx, endpoint, url, query)
	live.recordExecutions(ctx, execution.attempts)
	return execution.payload, execution.err
}

// requestOpenAlexSourceWorks preserves source normalization, explicit cursor and short-page termination under the caller lock.
func (live *LiveTransport) requestOpenAlexSourceWorks(ctx context.Context, request Request) (any, error) {
	sourceId := strings.TrimRight(strings.TrimSpace(request.SourceId), "/")
	if position := strings.LastIndex(sourceId, "/"); position >= 0 {
		sourceId = sourceId[position+1:]
	}
	if sourceId == "" {
		return map[string]any{"results": []any{}}, nil
	}
	filter := "primary_location.source.id:" + sourceId + ",type:article|book-chapter"
	if request.FromSyncDate != nil && strings.TrimSpace(*request.FromSyncDate) != "" {
		filter += ",from_created_date:" + *request.FromSyncDate
	}
	cursor := "*"
	if request.Cursor != nil {
		cursor = *request.Cursor
	}
	payload, err := live.openAlexRequest(ctx, "source_works", live.openAlexBase+"/works", []QueryPair{{"filter", filter}, {"per-page", "200"}, {"cursor", cursor}, {"sort", "publication_date:desc"}, {"select", openAlexWorkFields}})
	if err != nil {
		return nil, err
	}
	if len(array(field(payload, "results"))) < 200 {
		if meta, ok := field(payload, "meta").(map[string]any); ok {
			meta["next_cursor"] = nil
		}
	}
	return payload, nil
}
