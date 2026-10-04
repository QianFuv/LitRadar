package index

import (
	"fmt"
	"path/filepath"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/sources"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// LiveConfig receives resolved deployment settings without introducing defaults over explicit caller values.
type LiveConfig struct {
	ApplicationExecutable, ProjectRoot, SecretKeyFile                                                          string
	File, StopAfter                                                                                            *string
	WorkerCount, ProcessCount                                                                                  *uint64
	IssueBatchSize, TimeoutSeconds                                                                             uint64
	ShouldResume, ShouldUpdate, ShouldFullRescan, ShouldNotify, IsNotifyDryRun, ShouldAcknowledgeUnknownNotify bool
	ScholarlyConfig                                                                                            scholarly.LiveConfig
	CnkiCaptchaToken                                                                                           *string
	ProviderProxySelection                                                                                     sources.ProxySelection
	IndexProviderRoutes                                                                                        map[string]string
}

// Format omits credential and proxy values from every diagnostic representation.
func (value LiveConfig) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "LiveConfig{root:%q file:%v resume:%t update:%t full_rescan:%t notify:%t credentials:[REDACTED]}", value.ProjectRoot, value.File, value.ShouldResume, value.ShouldUpdate, value.ShouldFullRescan, value.ShouldNotify)
}

// LiveRunOutcome summarizes the ordered catalog boundaries reached by one invocation.
type LiveRunOutcome struct {
	Status  string               `json:"status"`
	Message *string              `json:"message"`
	Csvs    []LiveCatalogOutcome `json:"csvs"`
}

// LiveCatalogOutcome combines persisted execution counters with the current invocation's paths and concurrency.
type LiveCatalogOutcome struct {
	CsvPath             string          `json:"csv_path"`
	DbPath              string          `json:"db_path"`
	RunId               string          `json:"run_id"`
	Status              string          `json:"status"`
	JournalCount        uint64          `json:"journal_count"`
	WrittenArticleCount int64           `json:"written_article_count"`
	SourceAttemptCount  uint64          `json:"source_attempt_count"`
	Concurrency         LiveConcurrency `json:"concurrency"`
	ManifestPath        *string         `json:"manifest_path"`
	NotifyExitCode      *int32          `json:"notify_exit_code"`
}

// LiveConcurrency distinguishes configured capacity from executors actually started after resume filtering.
type LiveConcurrency struct {
	Provider                    string `json:"provider"`
	ConfiguredWorkers           uint64 `json:"configured_workers"`
	ConfiguredProcesses         uint64 `json:"configured_processes"`
	ConfiguredAggregateCapacity uint64 `json:"configured_aggregate_capacity"`
	AggregateLimit              uint64 `json:"aggregate_limit"`
	EffectiveWorkers            uint64 `json:"effective_workers"`
	ExecutorCount               uint64 `json:"executor_count"`
	ChildProcessCount           uint64 `json:"child_process_count"`
	InlineExecutorCount         uint64 `json:"inline_executor_count"`
	EffectiveAggregateCapacity  uint64 `json:"effective_aggregate_capacity"`
}

func catalogConcurrency(config LiveConfig, provider string) (LiveConcurrency, error) {
	capacity, err := indexdomain.ResolveConcurrency(config.WorkerCount, config.ProcessCount, provider == "scholarly")
	if err != nil {
		return LiveConcurrency{}, err
	}
	limit := uint64(32)
	if provider == "scholarly" {
		limit = 96
	}
	return LiveConcurrency{Provider: provider, ConfiguredWorkers: capacity.WorkerCount, ConfiguredProcesses: capacity.ProcessCount, ConfiguredAggregateCapacity: capacity.AggregateCapacity, AggregateLimit: limit}, nil
}
func (value *LiveConcurrency) setExecutors(count uint64, isChild bool) {
	value.ExecutorCount = count
	if isChild {
		value.ChildProcessCount = count
	} else {
		value.InlineExecutorCount = count
	}
	if count > 0 {
		value.EffectiveWorkers = 1
		if value.Provider == "scholarly" || value.Provider == "cnki" {
			value.EffectiveWorkers = value.ConfiguredWorkers
		}
	}
	value.EffectiveAggregateCapacity = value.EffectiveWorkers * count
}
func requestedMode(config LiveConfig) domain.IndexSyncMode {
	if config.ShouldFullRescan {
		return domain.FullRescan
	}
	if config.ShouldUpdate {
		return domain.Incremental
	}
	return domain.Bootstrap
}
func catalogDatabaseName(input storage.CatalogInput) string { return input.CatalogName + ".sqlite" }
func catalogContentPath(config LiveConfig, input storage.CatalogInput) string {
	return filepath.Join(config.ProjectRoot, "data", "index", catalogDatabaseName(input))
}
func catalogControlPath(config LiveConfig, name string) string {
	return filepath.Join(config.ProjectRoot, "data", "index-control", name+".sqlite")
}
func catalogManifestPath(input storage.CatalogInput) string {
	return filepath.Join("data", "push_state", input.CatalogName+".changes.json")
}
func catalogHistoryDirectory(config LiveConfig, input storage.CatalogInput) string {
	return filepath.Join(config.ProjectRoot, "data", "push_state", "history", input.CatalogName)
}
func catalogOutcome(config LiveConfig, input storage.CatalogInput, persisted storage.BatchCatalogOutcome, handoff *storage.NotifyHandoffState, concurrency LiveConcurrency) LiveCatalogOutcome {
	outcome := LiveCatalogOutcome{CsvPath: input.Path, DbPath: catalogContentPath(config, input), RunId: persisted.RunId, Status: "succeeded", JournalCount: persisted.JournalCount, WrittenArticleCount: persisted.WrittenArticleCount, SourceAttemptCount: persisted.SourceAttemptCount, Concurrency: concurrency}
	if persisted.ManifestPath != nil {
		path := filepath.Join(config.ProjectRoot, *persisted.ManifestPath)
		outcome.ManifestPath = &path
	}
	if handoff != nil {
		outcome.NotifyExitCode = handoff.ExitCode
	}
	return outcome
}
