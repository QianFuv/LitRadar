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
	if err := parseIndexWorkerCount(args, &configuration); err != nil {
		return configuration, false, err
	}
	isExplicit, err := parseIndexLegacyBatch(args, &configuration)
	if err != nil {
		return configuration, false, err
	}
	if err := parseIndexExecutionLimits(args, &configuration); err != nil {
		return configuration, isExplicit, err
	}
	configuration.ShouldResume = args.boolPair("--resume", "--no-resume", true)
	configuration.ShouldUpdate = args.boolPair("--update", "--no-update", false)
	configuration.ShouldFullRescan = args.boolPair("--full-rescan", "--no-full-rescan", false)
	configuration.ShouldNotify = args.boolPair("--notify", "--no-notify", false)
	configuration.IsNotifyDryRun = args.boolPair("--notify-dry-run", "--no-notify-dry-run", false)
	configuration.ShouldAcknowledgeUnknownNotify = args.flag("--acknowledge-unknown-notify")
	err = validateIndexModes(configuration)
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
	invocation, err := parseIndexInvocation(&args)
	if err != nil {
		return err
	}
	if invocation.isLegacyBatchExplicit {
		slog.WarnContext(ctx, "explicit --issue-batch is legacy resume metadata and does not control current Provider concurrency or memory", "event", "cli.index.legacy_issue_batch", "component", "cli", "option", "issue_batch", "behavior", "resume_compatibility_only")
	}
	key, err := requireKey(invocation.keyOption)
	if err != nil {
		return err
	}
	return executeIndexInvocation(ctx, bundle, invocation, key, executable, output)
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

// parseIndexWorkerCount preserves long-option precedence and zero rejection before legacy batch parsing.
func parseIndexWorkerCount(args *arguments, configuration *index.LiveConfig) error {
	workerFlag := "--workers"
	if !slices.Contains(*args, workerFlag) {
		workerFlag = "-w"
	}
	var err error
	configuration.WorkerCount, err = args.optionalUnsigned(workerFlag)
	if err != nil {
		return err
	}
	if configuration.WorkerCount != nil && *configuration.WorkerCount == 0 {
		return errors.New("--workers must be at least 1")
	}
	return nil
}

// parseIndexLegacyBatch retains independent defaults and explicitness on error.
func parseIndexLegacyBatch(args *arguments, configuration *index.LiveConfig) (bool, error) {
	batch, err := args.optionalUnsigned("--issue-batch")
	if err != nil {
		return false, err
	}
	isExplicit := batch != nil
	configuration.IssueBatchSize = 8
	if isExplicit {
		configuration.IssueBatchSize = *batch
	}
	if configuration.IssueBatchSize == 0 {
		return false, errors.New("--issue-batch must be at least 1")
	}
	return isExplicit, nil
}

// parseIndexExecutionLimits updates timeout and process values before validating requested concurrency.
func parseIndexExecutionLimits(args *arguments, configuration *index.LiveConfig) error {
	var err error
	configuration.TimeoutSeconds, err = args.unsigned("--timeout", 20)
	if err != nil {
		return err
	}
	configuration.ProcessCount, err = args.optionalUnsigned("--processes")
	if err != nil {
		return err
	}
	if configuration.ProcessCount != nil && *configuration.ProcessCount == 0 {
		return errors.New("--processes must be at least 1")
	}
	if err := indexdomain.ValidateConcurrencyOptions(configuration.WorkerCount, configuration.ProcessCount); err != nil {
		return err
	}
	return nil
}

// validateIndexModes preserves ordered incompatible-mode and acknowledgement dependencies.
func validateIndexModes(configuration index.LiveConfig) error {
	switch {
	case configuration.ShouldUpdate && configuration.ShouldFullRescan:
		return errors.New("--update cannot be combined with --full-rescan")
	case configuration.ShouldNotify && !configuration.ShouldUpdate:
		return errors.New("--notify requires --update")
	case configuration.ShouldAcknowledgeUnknownNotify && !configuration.ShouldNotify:
		return errors.New("--acknowledge-unknown-notify requires --notify")
	case configuration.ShouldAcknowledgeUnknownNotify && !configuration.ShouldResume:
		return errors.New("--acknowledge-unknown-notify requires --resume")
	}
	return nil
}

// indexInvocation retains parsed deployment and execution options without requiring a key early.
type indexInvocation struct {
	storage               config.Config
	keyOption             *string
	options               index.LiveConfig
	isLegacyBatchExplicit bool
}

// parseIndexInvocation retains storage, key-option, execution and leftover admission order.
func parseIndexInvocation(args *arguments) (indexInvocation, error) {
	storage, err := args.storage()
	if err != nil {
		return indexInvocation{}, err
	}
	keyOption, err := args.take("--secret-key-file")
	if err != nil {
		return indexInvocation{}, err
	}
	configuration, isLegacyBatchExplicit, err := parseIndex(args)
	if err != nil {
		return indexInvocation{}, err
	}
	if len(*args) != 0 {
		return indexInvocation{}, fmt.Errorf("unexpected index arguments: %s", strings.Join(*args, " "))
	}
	return indexInvocation{storage, keyOption, configuration, isLegacyBatchExplicit}, nil
}

// executeIndexInvocation owns codecs and repositories through live execution and result output.
func executeIndexInvocation(ctx context.Context, bundle string, invocation indexInvocation, key, executable string, output io.Writer) error {
	storage, configuration := invocation.storage, invocation.options
	if err := prepareIndexDeployment(ctx, storage, configuration.File, bundle); err != nil {
		return err
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
	if err := applyIndexRuntimeSettings(&configuration, stored, executable, storage.ProjectRoot, key); err != nil {
		return err
	}
	result, err := index.RunLiveIndex(ctx, configuration)
	if err != nil {
		return err
	}
	return writeResult(output, map[string]any{"status": result.Status, "message": result.Message, "csvs": result.Csvs, "effective_concurrency": concurrencyPayload(configuration, result)})
}

// prepareIndexDeployment preserves migration, selected-index preflight and managed metadata order.
func prepareIndexDeployment(ctx context.Context, storage config.Config, selected *string, bundle string) error {
	if _, err := authmigration.Migrate(ctx, storage.AuthDbPath); err != nil {
		return err
	}
	if err := preflightIndex(ctx, storage, selected); err != nil {
		return err
	}
	if bundle != "" {
		report, err := meta.Prepare(ctx, storage, bundle)
		if err != nil {
			return err
		}
		runtime.ReportManagedMeta(ctx, report, "index_startup")
	}
	return nil
}

// applyIndexRuntimeSettings retains route decoding, captcha fallback and provider proxy admission order.
func applyIndexRuntimeSettings(configuration *index.LiveConfig, stored []settings.Value, executable, root, key string) error {
	lookup := func(field string) string {
		for _, value := range stored {
			if value.Field == field {
				return value.Value
			}
		}
		return ""
	}
	configuration.ApplicationExecutable, configuration.ProjectRoot, configuration.SecretKeyFile = executable, root, key
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
	var err error
	configuration.ProviderProxySelection, err = sources.ProxySelectionFromRuntime(stored)
	if err != nil {
		return err
	}
	return nil
}
