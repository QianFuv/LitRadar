package index

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// RunLiveIndex performs frozen catalog admission and durable indexing/publication/notification recovery.
func RunLiveIndex(ctx context.Context, config LiveConfig) (LiveRunOutcome, error) {
	return runLiveIndex(ctx, config, runLiveCatalog, RunNotifyProcess)
}

// runLiveIndex completes configuration and frozen selection admission before owning a batch connection.
func runLiveIndex(ctx context.Context, config LiveConfig, run catalogRunner, notify notifyRunner) (LiveRunOutcome, error) {
	if err := validateLiveConfig(config); err != nil {
		return LiveRunOutcome{}, err
	}
	meta := filepath.Join(config.ProjectRoot, "data", "meta")
	if _, err := os.Stat(meta); err != nil {
		return LiveRunOutcome{}, invalidWorker("managed catalog directory does not exist: " + meta)
	}
	paths, err := catalogPaths(meta, config.File)
	if err != nil {
		return LiveRunOutcome{}, err
	}
	if len(paths) == 0 {
		message := "no canonical catalog CSV files were selected"
		return LiveRunOutcome{Status: "skipped", Message: &message, Csvs: []LiveCatalogOutcome{}}, nil
	}
	request, err := prepareLiveBatchRequest(config, paths)
	if err != nil {
		return LiveRunOutcome{}, err
	}
	stopAfter, err := liveBatchStopBoundary(config, request)
	if err != nil {
		return LiveRunOutcome{}, err
	}
	batchPath := filepath.Join(config.ProjectRoot, "data", "index-control", storage.BatchDatabaseFilename)
	connection, err := storage.OpenBatch(ctx, batchPath)
	if err != nil {
		return LiveRunOutcome{}, err
	}
	defer connection.Close()
	owner := storage.NewBatchOwnerId()
	admission, err := storage.AdmitBatch(ctx, connection.Conn, request, config.ShouldResume, owner, time.Now().Unix())
	if err != nil {
		return LiveRunOutcome{}, err
	}
	return runAdmittedLiveBatch(ctx, connection.Conn, config, request, admission, owner, batchPath, stopAfter, run, notify)
}

func validateLiveConfig(config LiveConfig) error {
	if err := indexdomain.ValidateConcurrencyOptions(config.WorkerCount, config.ProcessCount); err != nil {
		return err
	}
	checks := []struct {
		invalid bool
		message string
	}{{config.IssueBatchSize == 0, "issue_batch_size must be greater than zero"}, {config.TimeoutSeconds == 0, "timeout_seconds must be greater than zero"}, {config.ShouldUpdate && config.ShouldFullRescan, "--update cannot be combined with --full-rescan"}, {config.ShouldNotify && !config.ShouldUpdate, "--notify requires an update manifest"}, {config.ShouldAcknowledgeUnknownNotify && !config.ShouldNotify, "--acknowledge-unknown-notify requires --notify"}, {config.ShouldAcknowledgeUnknownNotify && !config.ShouldResume, "--acknowledge-unknown-notify requires --resume"}, {len(config.IndexProviderRoutes) == 0, "index_provider_routes must not be empty"}}
	for _, check := range checks {
		if check.invalid {
			return invalidWorker(check.message)
		}
	}
	return nil
}
func validateSelectedCatalogs(config LiveConfig, inputs []storage.CatalogInput) error {
	for _, input := range inputs {
		if _, err := catalogConcurrency(config, input.ProviderName); err != nil {
			return err
		}
		if input.ProviderName == "scholarly" {
			for _, check := range []struct {
				hasValue bool
				message  string
			}{{config.ScholarlyConfig.HasCrossrefMailto(), "Crossref mailto is required for scholarly indexing"}, {config.ScholarlyConfig.HasOpenAlexKey(), "OpenAlex API key is required for scholarly indexing"}, {config.ScholarlyConfig.HasSemanticScholarKey(), "Semantic Scholar API key is required for scholarly indexing"}} {
				if !check.hasValue {
					return invalidWorker(check.message)
				}
			}
		}
	}
	return nil
}
func catalogPaths(directory string, file *string) ([]string, error) {
	if file != nil {
		if frozenCatalogBasename(*file) != *file || !hasCsvExtension(*file) {
			return nil, invalidWorker("--file must be one CSV filename without directory components")
		}
		path := filepath.Join(directory, *file)
		if _, err := os.Stat(path); err != nil {
			return []string{}, nil
		}
		return []string{path}, nil
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		return nil, err
	}
	paths := []string{}
	for _, entry := range entries {
		if hasCsvExtension(entry.Name()) {
			paths = append(paths, filepath.Join(directory, entry.Name()))
		}
	}
	return paths, nil
}

func hasCsvExtension(name string) bool {
	return name != ".csv" && filepath.Ext(name) == ".csv"
}

type leaseHeartbeat struct {
	stop chan struct{}
	done chan struct{}
	once sync.Once
	err  error
}

func startLeaseHeartbeat(renew func() error, panicMessage string, interval time.Duration) *leaseHeartbeat {
	heartbeat := &leaseHeartbeat{stop: make(chan struct{}), done: make(chan struct{})}
	go func() {
		defer close(heartbeat.done)
		defer func() {
			if recover() != nil {
				heartbeat.err = fmt.Errorf("%s", panicMessage)
			}
		}()
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for {
			select {
			case <-heartbeat.stop:
				return
			case <-ticker.C:
				if err := renew(); err != nil {
					heartbeat.err = err
					return
				}
			}
		}
	}()
	return heartbeat
}
func (heartbeat *leaseHeartbeat) stopAndCheck() error {
	heartbeat.once.Do(func() { close(heartbeat.stop) })
	<-heartbeat.done
	return heartbeat.err
}

// prepareLiveBatchRequest freezes every selected route before validating provider configuration.
func prepareLiveBatchRequest(config LiveConfig, paths []string) (storage.BatchRequest, error) {
	inputs, err := freezeSelectedLiveCatalogs(config, paths)
	if err != nil {
		return storage.BatchRequest{}, err
	}
	if err := validateSelectedCatalogs(config, inputs); err != nil {
		return storage.BatchRequest{}, err
	}
	selection := "all"
	if config.File != nil {
		selection = "explicit_file"
	}
	request, err := storage.NewBatchRequest(inputs, selection, requestedMode(config), config.IssueBatchSize, config.ShouldNotify, config.IsNotifyDryRun)
	if err != nil {
		return storage.BatchRequest{}, err
	}
	return request, nil
}

// liveBatchStopBoundary resolves the exact selected filename before opening the batch database.
func liveBatchStopBoundary(config LiveConfig, request storage.BatchRequest) (int, error) {
	stopAfter := len(request.Catalogs) - 1
	if config.StopAfter != nil {
		stopAfter = -1
		for ordinal, input := range request.Catalogs {
			if input.Filename == *config.StopAfter {
				stopAfter = ordinal
				break
			}
		}
		if stopAfter < 0 {
			return 0, invalidWorker("--stop-after must name an exact CSV in the selected catalogs")
		}
	}
	return stopAfter, nil
}

// runAdmittedLiveBatch stops and joins the heartbeat before any final lease disposition.
func runAdmittedLiveBatch(ctx context.Context, connection *sql.Conn, config LiveConfig, request storage.BatchRequest, admission storage.BatchAdmission, owner, batchPath string, stopAfter int, run catalogRunner, notify notifyRunner) (LiveRunOutcome, error) {
	batch := admission.Batch
	if admission.IsAbandoning {
		replacement, err := replaceAbandoningLiveBatch(ctx, connection, config, batch, request, owner)
		if err != nil {
			storage.ReleaseBatchLease(context.Background(), connection, batch.BatchId, owner)
			return LiveRunOutcome{}, err
		}
		batch = replacement
	}
	heartbeat := startLeaseHeartbeat(func() error {
		separate, err := storage.OpenBatch(context.Background(), batchPath)
		if err != nil {
			return err
		}
		defer separate.Close()
		return storage.HeartbeatBatchLease(context.Background(), separate.Conn, batch.BatchId, owner, time.Now().Unix())
	}, "batch heartbeat thread panicked", 30*time.Second)
	defer heartbeat.stopAndCheck()
	outcomes, executionError := executeAdmittedLiveBatch(ctx, connection, config, request, batch, stopAfter, run, notify)
	heartbeatError := heartbeat.stopAndCheck()
	if executionError != nil || heartbeatError != nil {
		storage.ReleaseBatchLease(context.Background(), connection, batch.BatchId, owner)
		if executionError != nil {
			return LiveRunOutcome{}, executionError
		}
		return LiveRunOutcome{}, heartbeatError
	}
	return finishLiveBatch(ctx, connection, batch, owner, outcomes)
}

// replaceAbandoningLiveBatch closes each checkpoint connection before considering replacement.
func replaceAbandoningLiveBatch(ctx context.Context, connection *sql.Conn, config LiveConfig, batch storage.IndexBatch, request storage.BatchRequest, owner string) (storage.IndexBatch, error) {
	for _, catalog := range batch.Catalogs {
		control, err := storage.OpenControl(ctx, catalogControlPath(config, catalog.CatalogName))
		if err != nil {
			return storage.IndexBatch{}, err
		}
		_, err = storage.AbandonBatchCheckpoints(ctx, control.Conn, batch.BatchId)
		control.Close()
		if err != nil {
			return storage.IndexBatch{}, err
		}
	}
	return storage.ReplaceAbandoningBatch(ctx, connection, batch.BatchId, request, owner, time.Now().Unix())
}

// executeAdmittedLiveBatch adopts legacy state for every requested catalog before any stop boundary.
func executeAdmittedLiveBatch(ctx context.Context, connection *sql.Conn, config LiveConfig, request storage.BatchRequest, batch storage.IndexBatch, stopAfter int, run catalogRunner, notify notifyRunner) ([]LiveCatalogOutcome, error) {
	if config.ShouldResume {
		for _, input := range request.Catalogs {
			control, err := storage.OpenControl(ctx, catalogControlPath(config, input.CatalogName))
			if err != nil {
				return nil, err
			}
			_, err = storage.AdoptLegacyBatchState(ctx, control.Conn, input.CatalogName, input.ProviderName, batch.BatchId, request.Mode, request.Selection == "explicit_file")
			control.Close()
			if err != nil {
				return nil, err
			}
		}
	}
	return runBatchCatalogs(ctx, connection, config, request, batch, stopAfter, run, notify)
}

// finishLiveBatch preserves caller-context pause release and completion-error precedence.
func finishLiveBatch(ctx context.Context, connection *sql.Conn, batch storage.IndexBatch, owner string, outcomes []LiveCatalogOutcome) (LiveRunOutcome, error) {
	if len(outcomes) < len(batch.Catalogs) {
		if err := storage.ReleaseBatchLease(ctx, connection, batch.BatchId, owner); err != nil {
			return LiveRunOutcome{}, err
		}
		message := "Requested catalog boundary reached; resume without --stop-after to continue."
		return LiveRunOutcome{Status: "paused", Message: &message, Csvs: outcomes}, nil
	}
	if err := storage.CompleteBatch(ctx, connection, batch.BatchId, owner, time.Now().Unix()); err != nil {
		storage.ReleaseBatchLease(context.Background(), connection, batch.BatchId, owner)
		return LiveRunOutcome{}, err
	}
	return LiveRunOutcome{Status: "succeeded", Csvs: outcomes}, nil
}

// freezeSelectedLiveCatalogs admits UTF-8 catalog stems and their configured routes in selection order.
func freezeSelectedLiveCatalogs(config LiveConfig, paths []string) ([]storage.CatalogInput, error) {
	inputs := []storage.CatalogInput{}
	for _, path := range paths {
		filename := frozenCatalogBasename(path)
		name := filename
		if extension := filepath.Ext(name); extension != "" && extension != name {
			name = strings.TrimSuffix(name, extension)
		}
		if name == "" || !utf8.ValidString(name) {
			return nil, invalidWorker("catalog filename must have a UTF-8 stem")
		}
		route, exists := config.IndexProviderRoutes[name]
		if !exists {
			return nil, invalidWorker("index_provider_routes has no route for catalog " + name)
		}
		input, err := FreezeCatalog(path, route)
		if err != nil {
			return nil, err
		}
		inputs = append(inputs, input)
	}
	return inputs, nil
}
