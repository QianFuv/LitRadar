package index

import (
	"context"
	"database/sql"
	"fmt"
	"log/slog"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"time"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func runLiveCatalog(ctx context.Context, config LiveConfig, input storage.CatalogInput, batchId string) (LiveCatalogOutcome, error) {
	return runLiveCatalogWithFactory(ctx, config, input, batchId, LiveWorkerProvider)
}

// runLiveCatalogWithFactory owns catalog connections and captures one lease and schedule epoch.
func runLiveCatalogWithFactory(ctx context.Context, config LiveConfig, input storage.CatalogInput, batchId string, factory WorkerProviderFactory) (LiveCatalogOutcome, error) {
	concurrency, err := catalogConcurrency(config, input.ProviderName)
	if err != nil {
		return LiveCatalogOutcome{}, err
	}
	contentPath := catalogContentPath(config, input)
	controlPath := catalogControlPath(config, input.CatalogName)
	if err := os.MkdirAll(filepath.Dir(contentPath), 0777); err != nil {
		return LiveCatalogOutcome{}, err
	}
	if err := os.MkdirAll(filepath.Dir(controlPath), 0777); err != nil {
		return LiveCatalogOutcome{}, err
	}
	control, err := storage.OpenControl(ctx, controlPath)
	if err != nil {
		return LiveCatalogOutcome{}, err
	}
	defer control.Close()
	captured := time.Now()
	seconds := max(captured.Unix(), 0)
	epoch := uint64(max(captured.UnixMilli(), 0))
	runId := input.CatalogName + "-" + strconv.FormatInt(max(captured.UnixNano(), 0), 10)
	timestamp := strconv.FormatInt(seconds, 10)
	if err := storage.AcquireLease(ctx, control.Conn, input.CatalogName, input.ProviderName, runId, seconds); err != nil {
		return LiveCatalogOutcome{}, err
	}
	release := func() error {
		return storage.ReleaseLease(context.Background(), control.Conn, input.CatalogName, input.ProviderName, runId)
	}
	content, err := storage.OpenContent(ctx, contentPath)
	if err != nil {
		release()
		return LiveCatalogOutcome{}, fmt.Errorf("index database %s cannot be used: %w", contentPath, err)
	}
	defer content.Close()
	execution := liveCatalogExecution{ctx: ctx, config: config, input: input, content: content.Conn, control: control.Conn, contentPath: contentPath, controlPath: controlPath, writer: WriterContext{input.CatalogName, input.ProviderName, batchId, runId, timestamp}, epoch: epoch, concurrency: concurrency, release: release}
	return execution.run(factory)
}

func liveBootstrap(config LiveConfig, request WorkerRequest, workset string) WorkerBootstrap {
	bootstrap := WorkerBootstrap{ProtocolVersion: 8, WorkerId: request.WorkerId}
	if request.ProviderName == "cnki" {
		bootstrap.CnkiCaptchaToken = config.CnkiCaptchaToken
	}
	if value, hasProxy := config.ProviderProxySelection.ProxyUrlForProvider(request.ProviderName); hasProxy {
		bootstrap.ProviderProxyUrl = &value
	}
	if request.ProviderName == "scholarly" {
		copy := config.ScholarlyConfig
		bootstrap.ScholarlyConfig = &copy
		bootstrap.ScholarlyWorksetDir = &workset
	}
	return bootstrap
}

// indexEntries prepares frozen journal runs and accumulates only committed page metrics.
func indexEntries(ctx context.Context, content, control *sql.Conn, writer WriterContext, entries []domain.JournalCatalogEntry, mode domain.IndexSyncMode, shouldResume bool, implementation provider.IndexContent) (RunMetrics, error) {
	metrics := RunMetrics{JournalsTotal: uint64(len(entries))}
	heartbeat := func() error {
		return storage.HeartbeatLease(ctx, control, writer.CatalogName, writer.ProviderName, writer.RunId, time.Now().Unix())
	}
	for ordinal, entry := range entries {
		if err := heartbeat(); err != nil {
			return metrics, err
		}
		assignment := WorkerAssignment{JournalOrdinal: uint64(ordinal), Entry: entry, Mode: mode}
		preparation, err := storage.PrepareJournalSync(ctx, control, writer.syncRun(assignment), shouldResume, writer.Timestamp)
		if err != nil {
			return metrics, err
		}
		if preparation.ShouldSkip {
			metrics.JournalsResumed++
			continue
		}
		run := preparation.Checkpoint
		assignment.Mode = run.Mode
		assignment.CommittedAnchor = run.BaseAnchor
		if err := indexEntryPages(ctx, content, control, writer, assignment, run.TraversalCheckpoint, implementation, heartbeat, &metrics); err != nil {
			return metrics, err
		}
	}
	return metrics, nil
}
func emitMetrics(ctx context.Context, writer WriterContext, metrics RunMetrics, outcome string) {
	slog.InfoContext(ctx, "index.run.completed", "event", "index.run.completed", "component", "index", "run_id", writer.RunId, "catalog", writer.CatalogName, "provider", writer.ProviderName, "worker_id", "all", "outcome", outcome, "journals_total", metrics.JournalsTotal, "journals_succeeded", metrics.JournalsSucceeded, "journals_resumed", metrics.JournalsResumed, "journals_failed", metrics.JournalsFailed, "pages_committed", metrics.PagesCommitted, "articles_seen", metrics.ArticlesSeen, "articles_changed", metrics.ArticlesChanged, "identity_aliases_added", metrics.IdentityAliasesAdded, "change_events_emitted", metrics.ChangeEventsEmitted)
}

// liveCatalogExecution keeps lease release and connection ownership outside the execution strategy.
type liveCatalogExecution struct {
	ctx                      context.Context
	config                   LiveConfig
	input                    storage.CatalogInput
	content, control         *sql.Conn
	contentPath, controlPath string
	writer                   WriterContext
	epoch                    uint64
	concurrency              LiveConcurrency
	release                  func() error
}

// reconcileLiveCatalogIdentities rejects legacy aliases before reconciling canonical content identity.
func reconcileLiveCatalogIdentities(ctx context.Context, content, control *sql.Conn, input storage.CatalogInput, contentPath string) error {
	aliases := []string{}
	for _, entry := range input.Entries {
		aliases = append(aliases, entry.CatalogAliases...)
	}
	slices.Sort(aliases)
	aliases = slices.Compact(aliases)
	hasLegacy, err := storage.HasCatalogAliasSyncState(ctx, control, input.CatalogName, aliases)
	if err != nil {
		return err
	}
	if hasLegacy {
		return invalidWorker("legacy catalog alias has provider synchronization state")
	}
	if err := storage.ReconcileCatalogIdentities(ctx, content, input.Entries); err != nil {
		return fmt.Errorf("index database %s cannot be used: %w", contentPath, err)
	}

	return nil
}

// run preserves execution-before-heartbeat errors and finalization-before-release errors.
func (execution *liveCatalogExecution) run(factory WorkerProviderFactory) (LiveCatalogOutcome, error) {
	if err := reconcileLiveCatalogIdentities(execution.ctx, execution.content, execution.control, execution.input, execution.contentPath); err != nil {
		execution.release()
		return LiveCatalogOutcome{}, err
	}
	isMultiprocess := execution.concurrency.ConfiguredProcesses > 1 && len(execution.input.Entries) > 1
	workset, err := filepath.Abs(filepath.Join(execution.config.ProjectRoot, "data", "index-work", "scholarly"))
	if err != nil {
		execution.release()
		return LiveCatalogOutcome{}, err
	}
	metrics, executionError, heartbeatError := execution.execute(factory, workset, isMultiprocess)
	if executionError != nil {
		execution.release()
		execution.emitFailureMetrics()
		return LiveCatalogOutcome{}, executionError
	}
	if !isMultiprocess {
		execution.setInlineExecutors(metrics)
	}
	if heartbeatError != nil {
		execution.release()
		return LiveCatalogOutcome{}, heartbeatError
	}
	if err := execution.finalize(); err != nil {
		return LiveCatalogOutcome{}, err
	}
	emitMetrics(execution.ctx, execution.writer, metrics, "success")
	return LiveCatalogOutcome{CsvPath: execution.input.Path, DbPath: execution.contentPath, RunId: execution.writer.RunId, Status: "succeeded", JournalCount: uint64(len(execution.input.Entries)), WrittenArticleCount: int64(min(metrics.ArticlesChanged, uint64(math.MaxInt64))), SourceAttemptCount: metrics.PagesCommitted, Concurrency: execution.concurrency}, nil
}

// execute selects the existing worker strategy without changing the configured capacity.
func (execution *liveCatalogExecution) execute(factory WorkerProviderFactory, workset string, isMultiprocess bool) (RunMetrics, error, error) {
	if isMultiprocess {
		metrics, err := execution.runProcesses(workset)
		return metrics, err, nil
	}
	return execution.runInline(factory, workset)
}

// runProcesses freezes unfinished requests before starting any child executor.
func (execution *liveCatalogExecution) runProcesses(workset string) (RunMetrics, error) {
	var metrics RunMetrics
	var executionError error
	requests, prepared, err := PrepareWorkerRequests(execution.ctx, execution.control, execution.writer, execution.input.Entries, requestedMode(execution.config), execution.config.ShouldResume, indexdomain.Concurrency{WorkerCount: execution.concurrency.ConfiguredWorkers, ProcessCount: execution.concurrency.ConfiguredProcesses, AggregateCapacity: execution.concurrency.ConfiguredAggregateCapacity}, execution.epoch, execution.config.TimeoutSeconds)
	metrics = prepared
	executionError = err
	if err == nil {
		execution.concurrency.setExecutors(uint64(len(requests)), true)
		writer, err := NewParentWriter(execution.content, execution.control, execution.writer, execution.config.ShouldUpdate, requests, prepared)
		executionError = err
		if err == nil {
			metrics, executionError = RunWorkerProcesses(execution.ctx, writer, requests, WorkerProcessConfig{Executable: execution.config.ApplicationExecutable, RequestDirectory: filepath.Join(execution.config.ProjectRoot, "data", "index-control", "worker-requests"), Bootstrap: func(request WorkerRequest) WorkerBootstrap { return liveBootstrap(execution.config, request, workset) }})
		}
	}

	return metrics, executionError
}

// runInline closes provider resources on factory failure and panic, then joins its heartbeat.
func (execution *liveCatalogExecution) runInline(factory WorkerProviderFactory, workset string) (RunMetrics, error, error) {
	var metrics RunMetrics
	var executionError, heartbeatError error
	heartbeat := startLeaseHeartbeat(func() error {
		separate, err := storage.OpenControl(context.Background(), execution.controlPath)
		if err != nil {
			return err
		}
		defer separate.Close()
		return storage.HeartbeatLease(context.Background(), separate.Conn, execution.input.CatalogName, execution.input.ProviderName, execution.writer.RunId, time.Now().Unix())
	}, "index heartbeat thread panicked", 30*time.Second)
	defer heartbeat.stopAndCheck()
	request := WorkerRequest{ProtocolVersion: 8, CatalogName: execution.input.CatalogName, ProviderName: execution.input.ProviderName, RunId: execution.writer.RunId, ProcessCount: 1, SourceWorkerCount: execution.concurrency.ConfiguredWorkers, ScheduleEpochUnixMillis: execution.epoch, TimeoutSeconds: execution.config.TimeoutSeconds}
	implementation, closeProvider, err := factory(request, liveBootstrap(execution.config, request, workset))
	executionError = err
	func() {
		if closeProvider != nil {
			defer closeProvider()
		}
		if err == nil {
			metrics, executionError = indexEntries(execution.ctx, execution.content, execution.control, execution.writer, execution.input.Entries, requestedMode(execution.config), execution.config.ShouldResume, implementation)
		}
	}()
	heartbeatError = heartbeat.stopAndCheck()

	return metrics, executionError, heartbeatError
}

// emitFailureMetrics reports same-batch durable coverage after releasing the catalog lease.
func (execution *liveCatalogExecution) emitFailureMetrics() {
	state, err := storage.ReadBatchJournalState(execution.ctx, execution.control, execution.input.CatalogName, execution.input.ProviderName, execution.writer.BatchId)
	failed := RunMetrics{JournalsTotal: uint64(len(execution.input.Entries)), JournalsFailed: 1}
	if err == nil {
		failed = FailureMetrics(uint64(len(execution.input.Entries)), state.Completed, state.InFlight)
	}
	emitMetrics(execution.ctx, execution.writer, failed, "failure")
}

// setInlineExecutors records no executor when every journal was already complete.
func (execution *liveCatalogExecution) setInlineExecutors(metrics RunMetrics) {
	count := uint64(0)
	if metrics.JournalsTotal > metrics.JournalsResumed {
		count = 1
	}
	execution.concurrency.setExecutors(count, false)
}

// finalize discards bootstrap outbox events only after successful content optimization.
func (execution *liveCatalogExecution) finalize() error {
	finalizationError := storage.OptimizeContent(execution.ctx, execution.content)
	if finalizationError == nil && !execution.config.ShouldUpdate {
		_, finalizationError = storage.DiscardContentChangeEvents(execution.ctx, execution.content)
	}
	releaseError := execution.release()
	if finalizationError != nil {
		return finalizationError
	}
	return releaseError
}

// indexEntryPages validates continuation only after content and progress commit successfully.
func indexEntryPages(ctx context.Context, content, control *sql.Conn, writer WriterContext, assignment WorkerAssignment, checkpoint *string, implementation provider.IndexContent, heartbeat func() error, metrics *RunMetrics) error {
	entry := assignment.Entry
	seen := map[string]bool{}
	if checkpoint != nil {
		seen[*checkpoint] = true
	}
	for page := uint64(0); page < maximumProviderPages; page++ {
		if err := heartbeat(); err != nil {
			return err
		}
		batch, err := implementation.Fetch(ctx, entry, domain.IndexFetchContext{Mode: assignment.Mode, CommittedAnchor: assignment.CommittedAnchor, TraversalCheckpoint: checkpoint})
		if err != nil {
			return err
		}
		revision := fmt.Sprintf("%s:%s:%d", writer.RunId, entry.CatalogId, page)
		outcome, err := storage.CommitContentThenProgress(ctx, control, writer.syncRun(assignment), batch.Progress, writer.Timestamp, func() (storage.ContentWriteOutcome, error) {
			return storage.WriteContentBatchWithEvents(ctx, content, entry, batch, revision, writer.Timestamp, assignment.Mode == domain.Incremental)
		})
		if err != nil {
			return err
		}
		metrics.record(outcome)
		if batch.Progress.State == domain.Complete {
			metrics.JournalsSucceeded++
			break
		}
		checkpoint = batch.Progress.Checkpoint
		if err := validateInlineCheckpoint(entry, checkpoint, seen, page); err != nil {
			return err
		}
	}
	return nil
}

// validateInlineCheckpoint preserves missing, repetition and catalog-specific page-limit errors.
func validateInlineCheckpoint(entry domain.JournalCatalogEntry, checkpoint *string, seen map[string]bool, page uint64) error {
	if checkpoint == nil {
		return invalidWorker("provider checkpoint is missing")
	}
	if seen[*checkpoint] {
		return invalidWorker("index provider returned a repeated checkpoint")
	}
	seen[*checkpoint] = true
	if page+1 == maximumProviderPages {
		return invalidWorker("provider page limit exceeded for catalog entry " + entry.CatalogId)
	}
	return nil
}
