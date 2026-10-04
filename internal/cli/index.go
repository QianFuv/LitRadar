package cli

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	"github.com/QianFuv/LitRadar/internal/index"
	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/sources"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/meta"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	indexmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func (args *arguments) takeAny(names ...string) (*string, error) {
	for _, name := range names {
		value, err := args.take(name)
		if value != nil || err != nil {
			return value, err
		}
	}
	return nil, nil
}

func (args *arguments) boolPair(positive, negative string, fallback bool) bool {
	if args.flag(positive) {
		fallback = true
	}
	if args.flag(negative) {
		fallback = false
	}
	return fallback
}

func parseIndex(args *arguments) (index.LiveConfig, bool, error) {
	var configuration index.LiveConfig
	var err error
	configuration.File, err = args.takeAny("--file", "-f")
	if err != nil {
		return configuration, false, err
	}
	configuration.StopAfter, err = args.take("--stop-after")
	if err != nil {
		return configuration, false, err
	}
	workerFlag := "--workers"
	if !slices.Contains(*args, workerFlag) {
		workerFlag = "-w"
	}
	configuration.WorkerCount, err = args.optionalUnsigned(workerFlag)
	if err != nil {
		return configuration, false, err
	}
	if configuration.WorkerCount != nil && *configuration.WorkerCount == 0 {
		return configuration, false, errors.New("--workers must be at least 1")
	}
	batch, err := args.optionalUnsigned("--issue-batch")
	if err != nil {
		return configuration, false, err
	}
	isExplicit := batch != nil
	configuration.IssueBatchSize = 8
	if isExplicit {
		configuration.IssueBatchSize = *batch
	}
	if configuration.IssueBatchSize == 0 {
		return configuration, false, errors.New("--issue-batch must be at least 1")
	}
	configuration.TimeoutSeconds, err = args.unsigned("--timeout", 20)
	if err != nil {
		return configuration, isExplicit, err
	}
	configuration.ProcessCount, err = args.optionalUnsigned("--processes")
	if err != nil {
		return configuration, isExplicit, err
	}
	if configuration.ProcessCount != nil && *configuration.ProcessCount == 0 {
		return configuration, isExplicit, errors.New("--processes must be at least 1")
	}
	if err := indexdomain.ValidateConcurrencyOptions(configuration.WorkerCount, configuration.ProcessCount); err != nil {
		return configuration, isExplicit, err
	}
	configuration.ShouldResume = args.boolPair("--resume", "--no-resume", true)
	configuration.ShouldUpdate = args.boolPair("--update", "--no-update", false)
	configuration.ShouldFullRescan = args.boolPair("--full-rescan", "--no-full-rescan", false)
	configuration.ShouldNotify = args.boolPair("--notify", "--no-notify", false)
	configuration.IsNotifyDryRun = args.boolPair("--notify-dry-run", "--no-notify-dry-run", false)
	configuration.ShouldAcknowledgeUnknownNotify = args.flag("--acknowledge-unknown-notify")
	switch {
	case configuration.ShouldUpdate && configuration.ShouldFullRescan:
		err = errors.New("--update cannot be combined with --full-rescan")
	case configuration.ShouldNotify && !configuration.ShouldUpdate:
		err = errors.New("--notify requires --update")
	case configuration.ShouldAcknowledgeUnknownNotify && !configuration.ShouldNotify:
		err = errors.New("--acknowledge-unknown-notify requires --notify")
	case configuration.ShouldAcknowledgeUnknownNotify && !configuration.ShouldResume:
		err = errors.New("--acknowledge-unknown-notify requires --resume")
	}
	return configuration, isExplicit, err
}

func runIndex(ctx context.Context, values []string, executable string, output io.Writer) error {
	bundle, err := meta.DiscoverPackagedDirectory()
	if err != nil {
		return err
	}
	if hasHelp(values) {
		return writeResult(output, indexUsage)
	}
	args := arguments(slices.Clone(values))
	workerRequest, err := args.take("--live-worker-request")
	if err != nil {
		return err
	}
	if workerRequest != nil {
		if len(args) > 0 {
			return fmt.Errorf("unexpected index worker arguments: %s", strings.Join(args, " "))
		}
		return index.RunWorkerRequestFile(ctx, *workerRequest)
	}
	storage, err := args.storage()
	if err != nil {
		return err
	}
	keyOption, err := args.take("--secret-key-file")
	if err != nil {
		return err
	}
	configuration, isLegacyBatchExplicit, err := parseIndex(&args)
	if err != nil {
		return err
	}
	if len(args) != 0 {
		return fmt.Errorf("unexpected index arguments: %s", strings.Join(args, " "))
	}
	if isLegacyBatchExplicit {
		slog.WarnContext(ctx, "explicit --issue-batch is legacy resume metadata and does not control current Provider concurrency or memory", "event", "cli.index.legacy_issue_batch", "component", "cli", "option", "issue_batch", "behavior", "resume_compatibility_only")
	}
	key, err := requireKey(keyOption)
	if err != nil {
		return err
	}
	if _, err := authmigration.Migrate(ctx, storage.AuthDbPath); err != nil {
		return err
	}
	if err := preflightIndex(ctx, storage, configuration.File); err != nil {
		return err
	}
	if bundle != "" {
		report, err := meta.Prepare(ctx, storage, bundle)
		if err != nil {
			return err
		}
		runtime.ReportManagedMeta(ctx, report, "index_startup")
	}
	codec, err := loadVerifiedKey(ctx, storage.AuthDbPath, key)
	if err != nil {
		return err
	}
	defer codec.Close()
	repository, err := authstorage.Open(storage.AuthDbPath)
	if err != nil {
		return err
	}
	defer repository.Close()
	stored, err := settings.New(repository, codec).Load(ctx)
	if err != nil {
		return err
	}
	lookup := func(field string) string {
		for _, value := range stored {
			if value.Field == field {
				return value.Value
			}
		}
		return ""
	}
	configuration.ApplicationExecutable, configuration.ProjectRoot, configuration.SecretKeyFile = executable, storage.ProjectRoot, key
	configuration.ScholarlyConfig = scholarly.ConfigFromValuePools(configuration.TimeoutSeconds, lookup("openalex_api_key_pool"), lookup("semantic_scholar_api_key_pool"), lookup("crossref_mailto_pool"))
	if err := json.Unmarshal([]byte(lookup("index_provider_routes")), &configuration.IndexProviderRoutes); err != nil {
		return err
	}
	captcha := strings.TrimSpace(lookup("cnki_captcha_token"))
	if captcha == "" {
		captcha = os.Getenv("LITRADAR_CNKI_CAPTCHA_TOKEN")
	}
	if strings.TrimSpace(captcha) != "" {
		configuration.CnkiCaptchaToken = &captcha
	}
	configuration.ProviderProxySelection, err = sources.ProxySelectionFromRuntime(stored)
	if err != nil {
		return err
	}
	result, err := index.RunLiveIndex(ctx, configuration)
	if err != nil {
		return err
	}
	return writeResult(output, map[string]any{"status": result.Status, "message": result.Message, "csvs": result.Csvs, "effective_concurrency": concurrencyPayload(configuration, result)})
}

func preflightIndex(ctx context.Context, storage config.Config, selected *string) error {
	if selected == nil {
		return indexmigration.PreflightExisting(ctx, storage)
	}
	if filepath.Base(*selected) != *selected || *selected == ".csv" || filepath.Ext(*selected) != ".csv" {
		return errors.New("--file must be one CSV filename without directory components")
	}
	filename := filepath.Join(storage.IndexDir, strings.TrimSuffix(*selected, ".csv")+".sqlite")
	if _, err := os.Stat(filename); err == nil {
		_, err := indexmigration.Preflight(ctx, filename)
		return err
	}
	return nil
}

func concurrencyPayload(configuration index.LiveConfig, result index.LiveRunOutcome) any {
	var configured, effective index.LiveConcurrency
	for _, catalog := range result.Csvs {
		if catalog.Concurrency.ConfiguredAggregateCapacity >= configured.ConfiguredAggregateCapacity {
			configured = catalog.Concurrency
		}
		if catalog.Concurrency.EffectiveAggregateCapacity >= effective.EffectiveAggregateCapacity {
			effective = catalog.Concurrency
		}
	}
	return map[string]any{"workers": configured.ConfiguredWorkers, "processes": configured.ConfiguredProcesses, "requested_workers": configuration.WorkerCount, "requested_processes": configuration.ProcessCount, "issue_batch": configuration.IssueBatchSize, "configured_workers": configured.ConfiguredWorkers, "configured_processes": configured.ConfiguredProcesses, "configured_aggregate_capacity": configured.ConfiguredAggregateCapacity, "effective_workers": effective.EffectiveWorkers, "effective_processes": effective.ExecutorCount, "effective_aggregate_capacity": effective.EffectiveAggregateCapacity, "aggregate_limit": configured.AggregateLimit}
}
