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
	aliases := []string{}
	for _, entry := range input.Entries {
		aliases = append(aliases, entry.CatalogAliases...)
	}
	slices.Sort(aliases)
	aliases = slices.Compact(aliases)
	hasLegacy, err := storage.HasCatalogAliasSyncState(ctx, control.Conn, input.CatalogName, aliases)
	if err != nil {
		release()
		return LiveCatalogOutcome{}, err
	}
	if hasLegacy {
		release()
		return LiveCatalogOutcome{}, invalidWorker("legacy catalog alias has provider synchronization state")
	}
	if err := storage.ReconcileCatalogIdentities(ctx, content.Conn, input.Entries); err != nil {
		release()
		return LiveCatalogOutcome{}, fmt.Errorf("index database %s cannot be used: %w", contentPath, err)
	}
	writerContext := WriterContext{input.CatalogName, input.ProviderName, batchId, runId, timestamp}
	isMultiprocess := concurrency.ConfiguredProcesses > 1 && len(input.Entries) > 1
	var metrics RunMetrics
	var executionError, heartbeatError error
	workset, err := filepath.Abs(filepath.Join(config.ProjectRoot, "data", "index-work", "scholarly"))
	if err != nil {
		release()
		return LiveCatalogOutcome{}, err
	}
	if isMultiprocess {
		requests, prepared, err := PrepareWorkerRequests(ctx, control.Conn, writerContext, input.Entries, requestedMode(config), config.ShouldResume, indexdomain.Concurrency{WorkerCount: concurrency.ConfiguredWorkers, ProcessCount: concurrency.ConfiguredProcesses, AggregateCapacity: concurrency.ConfiguredAggregateCapacity}, epoch, config.TimeoutSeconds)
		metrics = prepared
		executionError = err
		if err == nil {
			concurrency.setExecutors(uint64(len(requests)), true)
			writer, err := NewParentWriter(content.Conn, control.Conn, writerContext, config.ShouldUpdate, requests, prepared)
			executionError = err
			if err == nil {
				metrics, executionError = RunWorkerProcesses(ctx, writer, requests, WorkerProcessConfig{Executable: config.ApplicationExecutable, RequestDirectory: filepath.Join(config.ProjectRoot, "data", "index-control", "worker-requests"), Bootstrap: func(request WorkerRequest) WorkerBootstrap { return liveBootstrap(config, request, workset) }})
			}
		}
	} else {
		heartbeat := startLeaseHeartbeat(func() error {
			separate, err := storage.OpenControl(context.Background(), controlPath)
			if err != nil {
				return err
			}
			defer separate.Close()
			return storage.HeartbeatLease(context.Background(), separate.Conn, input.CatalogName, input.ProviderName, runId, time.Now().Unix())
		}, "index heartbeat thread panicked", 30*time.Second)
		defer heartbeat.stopAndCheck()
		request := WorkerRequest{ProtocolVersion: 8, CatalogName: input.CatalogName, ProviderName: input.ProviderName, RunId: runId, ProcessCount: 1, SourceWorkerCount: concurrency.ConfiguredWorkers, ScheduleEpochUnixMillis: epoch, TimeoutSeconds: config.TimeoutSeconds}
		implementation, closeProvider, err := factory(request, liveBootstrap(config, request, workset))
		executionError = err
		func() {
			if closeProvider != nil {
				defer closeProvider()
			}
			if err == nil {
				metrics, executionError = indexEntries(ctx, content.Conn, control.Conn, writerContext, input.Entries, requestedMode(config), config.ShouldResume, implementation)
			}
		}()
		heartbeatError = heartbeat.stopAndCheck()
	}
	if executionError != nil {
		release()
		state, err := storage.ReadBatchJournalState(ctx, control.Conn, input.CatalogName, input.ProviderName, batchId)
		failed := RunMetrics{JournalsTotal: uint64(len(input.Entries)), JournalsFailed: 1}
		if err == nil {
			failed = FailureMetrics(uint64(len(input.Entries)), state.Completed, state.InFlight)
		}
		emitMetrics(ctx, writerContext, failed, "failure")
		return LiveCatalogOutcome{}, executionError
	}
	if !isMultiprocess {
		count := uint64(0)
		if metrics.JournalsTotal > metrics.JournalsResumed {
			count = 1
		}
		concurrency.setExecutors(count, false)
	}
	if heartbeatError != nil {
		release()
		return LiveCatalogOutcome{}, heartbeatError
	}
	finalizationError := storage.OptimizeContent(ctx, content.Conn)
	if finalizationError == nil && !config.ShouldUpdate {
		_, finalizationError = storage.DiscardContentChangeEvents(ctx, content.Conn)
	}
	releaseError := release()
	if finalizationError != nil {
		return LiveCatalogOutcome{}, finalizationError
	}
	if releaseError != nil {
		return LiveCatalogOutcome{}, releaseError
	}
	emitMetrics(ctx, writerContext, metrics, "success")
	return LiveCatalogOutcome{CsvPath: input.Path, DbPath: contentPath, RunId: runId, Status: "succeeded", JournalCount: uint64(len(input.Entries)), WrittenArticleCount: int64(min(metrics.ArticlesChanged, uint64(math.MaxInt64))), SourceAttemptCount: metrics.PagesCommitted, Concurrency: concurrency}, nil
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
		checkpoint := run.TraversalCheckpoint
		seen := map[string]bool{}
		if checkpoint != nil {
			seen[*checkpoint] = true
		}
		for page := uint64(0); page < maximumProviderPages; page++ {
			if err := heartbeat(); err != nil {
				return metrics, err
			}
			batch, err := implementation.Fetch(ctx, entry, domain.IndexFetchContext{Mode: run.Mode, CommittedAnchor: run.BaseAnchor, TraversalCheckpoint: checkpoint})
			if err != nil {
				return metrics, err
			}
			revision := fmt.Sprintf("%s:%s:%d", writer.RunId, entry.CatalogId, page)
			outcome, err := storage.CommitContentThenProgress(ctx, control, writer.syncRun(assignment), batch.Progress, writer.Timestamp, func() (storage.ContentWriteOutcome, error) {
				return storage.WriteContentBatchWithEvents(ctx, content, entry, batch, revision, writer.Timestamp, run.Mode == domain.Incremental)
			})
			if err != nil {
				return metrics, err
			}
			metrics.record(outcome)
			if batch.Progress.State == domain.Complete {
				metrics.JournalsSucceeded++
				break
			}
			checkpoint = batch.Progress.Checkpoint
			if checkpoint == nil {
				return metrics, invalidWorker("provider checkpoint is missing")
			}
			if seen[*checkpoint] {
				return metrics, invalidWorker("index provider returned a repeated checkpoint")
			}
			seen[*checkpoint] = true
			if page+1 == maximumProviderPages {
				return metrics, invalidWorker("provider page limit exceeded for catalog entry " + entry.CatalogId)
			}
		}
	}
	return metrics, nil
}
func emitMetrics(ctx context.Context, writer WriterContext, metrics RunMetrics, outcome string) {
	slog.InfoContext(ctx, "index.run.completed", "event", "index.run.completed", "component", "index", "run_id", writer.RunId, "catalog", writer.CatalogName, "provider", writer.ProviderName, "worker_id", "all", "outcome", outcome, "journals_total", metrics.JournalsTotal, "journals_succeeded", metrics.JournalsSucceeded, "journals_resumed", metrics.JournalsResumed, "journals_failed", metrics.JournalsFailed, "pages_committed", metrics.PagesCommitted, "articles_seen", metrics.ArticlesSeen, "articles_changed", metrics.ArticlesChanged, "identity_aliases_added", metrics.IdentityAliasesAdded, "change_events_emitted", metrics.ChangeEventsEmitted)
}
