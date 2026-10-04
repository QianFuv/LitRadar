package index

import (
	"context"
	"database/sql"
	"fmt"
	"io"
	"math"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// RunMetrics counts committed work without storing observability in canonical content tables.
type RunMetrics struct {
	JournalsTotal        uint64 `json:"journals_total"`
	JournalsSucceeded    uint64 `json:"journals_succeeded"`
	JournalsResumed      uint64 `json:"journals_resumed"`
	JournalsFailed       uint64 `json:"journals_failed"`
	PagesCommitted       uint64 `json:"pages_committed"`
	ArticlesSeen         uint64 `json:"articles_seen"`
	ArticlesChanged      uint64 `json:"articles_changed"`
	IdentityAliasesAdded uint64 `json:"identity_aliases_added"`
	ChangeEventsEmitted  uint64 `json:"change_events_emitted"`
}

// FailureMetrics uses durable same-batch coverage and reports only the current journal failure.
func FailureMetrics(total, completed, inFlight uint64) RunMetrics {
	metrics := RunMetrics{JournalsTotal: total, JournalsSucceeded: min(total, completed)}
	if inFlight > 0 || metrics.JournalsSucceeded < total {
		metrics.JournalsFailed = 1
	}
	return metrics
}
func (metrics *RunMetrics) record(outcome storage.ContentWriteOutcome) {
	metrics.PagesCommitted++
	metrics.ArticlesSeen += outcome.ArticlesSeen
	metrics.ArticlesChanged += outcome.ArticlesChanged
	metrics.IdentityAliasesAdded += outcome.IdentityAliasesAdded
	metrics.ChangeEventsEmitted += outcome.ChangeEventsEmitted
}

// WriterContext freezes the shared catalog ownership and safe timestamp for all page commits.
type WriterContext struct{ CatalogName, ProviderName, BatchId, RunId, Timestamp string }

func (value WriterContext) syncRun(assignment WorkerAssignment) storage.SyncRun {
	return storage.SyncRun{Scope: storage.SyncScope{CatalogName: value.CatalogName, ProviderName: value.ProviderName, CatalogId: assignment.Entry.CatalogId}, BatchId: value.BatchId, RunId: value.RunId, Mode: assignment.Mode, BaseAnchor: assignment.CommittedAnchor}
}

// PrepareWorkerRequests freezes all journal runs and partitions only unfinished work while retaining original ordinals.
func PrepareWorkerRequests(ctx context.Context, control *sql.Conn, writer WriterContext, entries []domain.JournalCatalogEntry, mode domain.IndexSyncMode, shouldResume bool, concurrency indexdomain.Concurrency, epoch, timeout uint64) ([]WorkerRequest, RunMetrics, error) {
	metrics := RunMetrics{JournalsTotal: uint64(len(entries))}
	assignments := []WorkerAssignment{}
	for ordinal, entry := range entries {
		assignment := WorkerAssignment{JournalOrdinal: uint64(ordinal), Entry: entry, Mode: mode}
		preparation, err := storage.PrepareJournalSync(ctx, control, writer.syncRun(assignment), shouldResume, writer.Timestamp)
		if err != nil {
			return nil, metrics, err
		}
		if preparation.ShouldSkip {
			metrics.JournalsResumed++
			continue
		}
		run := preparation.Checkpoint
		assignment.Mode = run.Mode
		assignment.CommittedAnchor = run.BaseAnchor
		assignment.TraversalCheckpoint = run.TraversalCheckpoint
		assignments = append(assignments, assignment)
	}
	if len(assignments) == 0 {
		return []WorkerRequest{}, metrics, nil
	}
	count := max(uint64(1), min(concurrency.ProcessCount, uint64(len(assignments))))
	requests := make([]WorkerRequest, int(count))
	for ordinal := range requests {
		requests[ordinal] = WorkerRequest{ProtocolVersion: WorkerProtocolVersion, CatalogName: writer.CatalogName, ProviderName: writer.ProviderName, RunId: writer.RunId, WorkerId: uint64(ordinal), ProcessCount: count, SourceWorkerCount: concurrency.WorkerCount, ScheduleEpochUnixMillis: epoch, TimeoutSeconds: timeout, Assignments: []WorkerAssignment{}}
	}
	for ordinal, assignment := range assignments {
		partition := ordinal % len(requests)
		requests[partition].Assignments = append(requests[partition].Assignments, assignment)
	}
	return requests, metrics, nil
}

type writerProgress struct {
	assignments            []WorkerAssignment
	position               int
	nextSequence, nextPage uint64
	hasTerminal            bool
}

// ParentWriter serializes canonical writes and emits ACK only after content and checkpoint commits.
type ParentWriter struct {
	content, control   *sql.Conn
	context            WriterContext
	shouldRecordEvents bool
	progress           []writerProgress
	Metrics            RunMetrics
}

// NewParentWriter validates pipe-bound identities and disjoint assignments before any child is started.
func NewParentWriter(content, control *sql.Conn, context WriterContext, shouldRecordEvents bool, requests []WorkerRequest, metrics RunMetrics) (*ParentWriter, error) {
	progress := make([]writerProgress, len(requests))
	ordinals := map[uint64]bool{}
	for ordinal, request := range requests {
		if request.ProtocolVersion != WorkerProtocolVersion || request.WorkerId != uint64(ordinal) || request.ProcessCount != uint64(len(requests)) || request.CatalogName != context.CatalogName || request.ProviderName != context.ProviderName || request.RunId != context.RunId || len(request.Assignments) == 0 {
			return nil, parentProtocolFailure(uint64(ordinal))
		}
		for _, assignment := range request.Assignments {
			if ordinals[assignment.JournalOrdinal] {
				return nil, parentProtocolFailure(uint64(ordinal))
			}
			ordinals[assignment.JournalOrdinal] = true
		}
		progress[ordinal].assignments = request.Assignments
	}
	return &ParentWriter{content, control, context, shouldRecordEvents, progress, metrics}, nil
}
func parentProtocolFailure(worker uint64) error {
	return parentFailure(worker, WorkerFailure{Class: "worker", Operation: "worker_protocol"})
}
func parentFailure(worker uint64, failure WorkerFailure) error {
	return &ExecutionError{Failure: failure, Message: fmt.Sprintf("worker %d failed during %s (%s)", worker, failure.Operation, failure.Class)}
}

// Handle validates one pipe's next frame, commits its page and advances memory only after ACK flush.
func (writer *ParentWriter) Handle(ctx context.Context, pipeWorker uint64, message WorkerMessage, acknowledgements io.Writer) error {
	if pipeWorker >= uint64(len(writer.progress)) {
		return parentProtocolFailure(pipeWorker)
	}
	progress := &writer.progress[pipeWorker]
	if progress.hasTerminal || message.ProtocolVersion != WorkerProtocolVersion || message.WorkerId != pipeWorker || message.Sequence != progress.nextSequence {
		return parentProtocolFailure(pipeWorker)
	}
	switch message.Type {
	case "batch":
		if message.Batch == nil || message.PageIndex != progress.nextPage || message.PageIndex >= maximumProviderPages || progress.position >= len(progress.assignments) {
			return parentProtocolFailure(pipeWorker)
		}
		assignment := progress.assignments[progress.position]
		batch := *message.Batch
		if message.JournalOrdinal != assignment.JournalOrdinal || batch.CatalogId != assignment.Entry.CatalogId {
			return parentProtocolFailure(pipeWorker)
		}
		revision := fmt.Sprintf("%s:%s:%d", writer.context.RunId, assignment.Entry.CatalogId, message.PageIndex)
		outcome, err := storage.CommitContentThenProgress(ctx, writer.control, writer.context.syncRun(assignment), batch.Progress, writer.context.Timestamp, func() (storage.ContentWriteOutcome, error) {
			return storage.WriteContentBatchWithEvents(ctx, writer.content, assignment.Entry, batch, revision, writer.context.Timestamp, writer.shouldRecordEvents)
		})
		if err != nil {
			return err
		}
		writer.Metrics.record(outcome)
		isComplete := batch.Progress.State == domain.Complete
		if acknowledgements == nil {
			return parentProtocolFailure(pipeWorker)
		}
		if err := WriteProtocol(acknowledgements, ParentMessage{"committed", WorkerProtocolVersion, pipeWorker, message.Sequence, message.JournalOrdinal, message.PageIndex, isComplete}); err != nil {
			return parentProtocolFailure(pipeWorker)
		}
		if progress.nextSequence == math.MaxUint64 {
			return parentProtocolFailure(pipeWorker)
		}
		progress.nextSequence++
		if isComplete {
			progress.position++
			progress.nextPage = 0
			writer.Metrics.JournalsSucceeded++
		} else {
			progress.nextPage++
		}
	case "succeeded":
		if progress.position != len(progress.assignments) {
			return parentProtocolFailure(pipeWorker)
		}
		progress.hasTerminal = true
	case "failed":
		if message.Failure == nil {
			return parentProtocolFailure(pipeWorker)
		}
		return parentFailure(pipeWorker, *message.Failure)
	default:
		return parentProtocolFailure(pipeWorker)
	}
	return nil
}

// End validates the terminal/EOF/exit combination after the process supervisor has reaped the leader.
func (writer *ParentWriter) End(worker uint64, exitError error) error {
	if worker >= uint64(len(writer.progress)) || !writer.progress[worker].hasTerminal {
		return parentProtocolFailure(worker)
	}
	if exitError != nil {
		return parentFailure(worker, WorkerFailure{Class: "worker", Operation: "worker_process"})
	}
	return nil
}
