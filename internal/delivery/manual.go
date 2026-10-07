package delivery

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/recommend"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// ManualWeeklyPushConfig keeps the original source window and shared execution boundary across databases.
type ManualWeeklyPushConfig struct {
	WindowEnd           weekly.Timestamp
	StorageConfig       storageconfig.Config
	SecretCodec         *secrets.Codec
	UserId              int64
	AttemptId           string
	AiModel             *string
	MaxCandidates       *int
	TimeoutSeconds      uint64
	RetryAttempts       int
	DedupeRetentionDays int64
	ExecutionControl    *domain.ExecutionControl
}

func (config ManualWeeklyPushConfig) String() string       { return "ManualWeeklyPushConfig([REDACTED])" }
func (config ManualWeeklyPushConfig) GoString() string     { return config.String() }
func (config ManualWeeklyPushConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

// ManualWeeklyPushOutcome is the persisted public summary of a manual recommendation job.
type ManualWeeklyPushOutcome struct {
	Status          string  `json:"status"`
	Message         string  `json:"message"`
	Pushed          int64   `json:"pushed"`
	Selected        int64   `json:"selected"`
	TotalCandidates *int64  `json:"total_candidates"`
	Summary         string  `json:"summary"`
	FolderId        *int64  `json:"folder_id"`
	FolderName      *string `json:"folder_name"`
}

func (outcome ManualWeeklyPushOutcome) String() string   { return "ManualWeeklyPushOutcome([REDACTED])" }
func (outcome ManualWeeklyPushOutcome) GoString() string { return outcome.String() }
func (outcome ManualWeeklyPushOutcome) LogValue() slog.Value {
	return slog.StringValue(outcome.String())
}

// RunManualWeeklyPush selects current settings against a fixed weekly publication window.
func RunManualWeeklyPush(ctx context.Context, config ManualWeeklyPushConfig) (ManualWeeklyPushOutcome, error) {
	return runManualWeeklyPush(ctx, config, runRecommendationDelivery)
}

func runManualWeeklyPush(ctx context.Context, config ManualWeeklyPushConfig, deliver func(context.Context, RunConfig, *int64, *recommend.ChangeManifest) (RunOutcome, error)) (result ManualWeeklyPushOutcome, resultError error) {
	ctx = observability.StartSpan(ctx, "litradar_worker::delivery::orchestration", "delivery.manual", map[string]any{"component": "delivery", "workflow": "manual_weekly_push", "mode": "execute", "user_id": config.UserId})
	started := time.Now()
	logger := slog.Default().With("component", "delivery", "workflow", "manual_weekly_push", "mode", "execute", "user_id", config.UserId)
	logger.InfoContext(ctx, "delivery.manual.started", "event", "delivery.manual.started", "outcome", "started")
	defer func() {
		if resultError != nil {
			logger.WarnContext(ctx, "delivery.manual.failed", "event", "delivery.manual.failed", "outcome", "failure", "status", "error", "error_kind", deliveryErrorKind(resultError), "duration_ms", time.Since(started).Milliseconds())
			return
		}
		count := int64(0)
		if result.TotalCandidates != nil {
			count = *result.TotalCandidates
		}
		attributes := []any{"status", result.Status, "selected_count", result.Selected, "delivered_count", result.Pushed, "candidate_count", count, "duration_ms", time.Since(started).Milliseconds()}
		if result.Status == "failed" || result.Status == "unknown" {
			logger.WarnContext(ctx, "delivery.manual.failed", append(attributes, "event", "delivery.manual.failed", "outcome", "failure")...)
		} else {
			logger.InfoContext(ctx, "delivery.manual.completed", append(attributes, "event", "delivery.manual.completed", "outcome", "success")...)
		}
	}()
	if err := checkManualExecution(config); err != nil {
		return result, err
	}
	repository, err := store.Open(config.StorageConfig.AuthDbPath)
	if err != nil {
		return result, classified("business_storage", err)
	}
	defer repository.Close()
	preferences, err := repository.GetNotificationSettings(ctx, config.SecretCodec, config.UserId)
	if err != nil {
		return result, classified("business_storage", err)
	}
	if preferences == nil || !preferences.Enabled {
		return manualOutcome("Recommendation settings are not enabled; skipped push", nil), nil
	}
	method := strings.TrimSpace(preferences.DeliveryMethod)
	if method == "" {
		method = "folder"
	}
	accounts, err := auth.Open(config.StorageConfig.AuthDbPath)
	if err != nil {
		return result, classified("business_storage", err)
	}
	defer accounts.Close()
	folder, err := favorites.New(accounts).TrackingFolder(ctx, identity.Id(config.UserId))
	if err != nil {
		return result, classified("business_storage", err)
	}
	if err := validateManualTrackingFolder(method, preferences, folder); err != nil {
		return result, err
	}
	return deliverManualWeeklyWindow(ctx, config, method, folder, preferences, deliver)
}

func manualOutcome(message string, folder *favorites.Folder) ManualWeeklyPushOutcome {
	result := ManualWeeklyPushOutcome{Status: "completed", Message: message}
	if folder != nil {
		result.FolderId = &folder.Id
		result.FolderName = &folder.Name
	}
	return result
}

func manualOutcomeFromDelivery(method string, folder *favorites.Folder, outcomes []RunOutcome) ManualWeeklyPushOutcome {
	result := manualOutcome("", folder)
	totals := collectManualDeliveryOutcomes(&result, outcomes)
	result.TotalCandidates = &totals.candidates
	if len(totals.failures) > 0 {
		result.Status = "failed"
		if totals.hasUnknown {
			result.Status = "unknown"
		}
		result.Message = strings.Join(totals.failures, "; ")
		return result
	}
	if result.Selected > 0 && method == "pushplus" {
		suffix := func(count int64) string {
			if count == 1 {
				return ""
			}
			return "s"
		}
		result.Message = fmt.Sprintf("PushPlus sent successfully (%d message%s); selected %d article%s across %d database%s", totals.messages, suffix(totals.messages), result.Selected, suffix(result.Selected), len(totals.selectedDatabases), suffix(int64(len(totals.selectedDatabases))))
		if result.Pushed > 0 {
			result.Message += fmt.Sprintf("; synced %d article%s to the tracking folder", result.Pushed, suffix(result.Pushed))
		}
	} else if result.Selected == 0 {
		result.Message = "AI selection found no matching articles"
		if len(totals.skips) > 0 {
			result.Message = totals.skips[0]
		}
	}
	return result
}

func validateManualTrackingFolder(method string, preferences *domain.NotificationSettings, folder *favorites.Folder) error {
	if (method == "folder" || preferences.SyncToTrackingFolder) && folder == nil {
		return errors.New("No tracking folder configured. Create a folder and set it as tracking first.")
	}
	return nil
}

func deliverManualWeeklyWindow(ctx context.Context, config ManualWeeklyPushConfig, method string, folder *favorites.Folder, preferences *domain.NotificationSettings, deliver func(context.Context, RunConfig, *int64, *recommend.ChangeManifest) (RunOutcome, error)) (ManualWeeklyPushOutcome, error) {
	publications, err := weekly.LoadAvailable(ctx, config.StorageConfig, config.WindowEnd, preferences.SelectedDatabases)
	if err != nil {
		return ManualWeeklyPushOutcome{}, classified("index_storage", err)
	}
	manifests := []weekly.Manifest{}
	for _, publication := range publications {
		if recommend.IsDatabaseSelected(preferences.SelectedDatabases, publication.DbName) {
			manifests = append(manifests, publication)
		}
	}
	if len(manifests) == 0 {
		message := "No new weekly articles available"
		if len(preferences.SelectedDatabases) > 0 {
			message = "No new weekly articles available in selected databases"
		}
		return manualOutcome(message, folder), nil
	}
	if len(preferences.Keywords) == 0 && len(preferences.Directions) == 0 {
		return manualOutcome("No keywords or directions configured; skipped push", folder), nil
	}
	workflow := store.WorkflowPush
	if method == "pushplus" {
		workflow = store.WorkflowNotify
	}
	outcomes, err := executeManualPublications(ctx, config, workflow, manifests, deliver)
	if err != nil {
		return ManualWeeklyPushOutcome{}, err
	}
	return manualOutcomeFromDelivery(method, folder, outcomes), nil
}

func executeManualPublications(ctx context.Context, config ManualWeeklyPushConfig, workflow store.Workflow, manifests []weekly.Manifest, deliver func(context.Context, RunConfig, *int64, *recommend.ChangeManifest) (RunOutcome, error)) ([]RunOutcome, error) {
	outcomes := []RunOutcome{}
	for _, publication := range manifests {
		if config.ExecutionControl != nil {
			if err := config.ExecutionControl.Check(); err != nil {
				return nil, err
			}
		}
		filename, err := config.StorageConfig.ResolveIndexDbPath(&publication.DbName)
		if err != nil {
			return nil, classified("index_storage", err)
		}
		child := RunConfig{AuthDbPath: config.StorageConfig.AuthDbPath, SecretCodec: config.SecretCodec, IndexDbPath: filename, DbName: publication.DbName, AttemptId: &config.AttemptId, AiModel: config.AiModel, MaxCandidates: config.MaxCandidates, TimeoutSeconds: config.TimeoutSeconds, RetryAttempts: config.RetryAttempts, DedupeRetentionDays: config.DedupeRetentionDays, Mode: store.RunModeExecute, Workflow: workflow, Trigger: store.TriggerKindScheduled, ExecutionControl: config.ExecutionControl}
		manifest := recommend.ChangeManifest{PendingIssueKeys: []string{}, PendingInpressKeys: []string{}, PendingArticleIds: publication.ArticleIds, RunId: publication.RunId}
		outcome, err := deliver(ctx, child, &config.UserId, &manifest)
		if err != nil {
			return nil, err
		}
		outcomes = append(outcomes, outcome)
	}
	return outcomes, nil
}

type manualDeliveryTotals struct {
	candidates, messages int64
	selectedDatabases    map[string]bool
	failures, skips      []string
	hasUnknown           bool
}

func collectManualDeliveryOutcomes(result *ManualWeeklyPushOutcome, outcomes []RunOutcome) manualDeliveryTotals {
	totals := manualDeliveryTotals{selectedDatabases: map[string]bool{}, failures: []string{}, skips: []string{}}
	for _, outcome := range outcomes {
		totals.candidates += int64(len(outcome.CandidateArticleIds))
		if outcome.Status == "failed" {
			totals.failures = append(totals.failures, outcome.DbName+" delivery failed")
		} else if outcome.Status == "unknown" {
			totals.hasUnknown = true
			totals.failures = append(totals.failures, outcome.DbName+" delivery outcome is unknown")
		}
		for _, subscriber := range outcome.Subscribers {
			collectManualSubscriber(result, &totals, outcome.DbName, subscriber)
		}
	}
	return totals
}

func collectManualSubscriber(result *ManualWeeklyPushOutcome, totals *manualDeliveryTotals, database string, subscriber SubscriberPlan) {
	result.Selected += int64(len(subscriber.SelectedArticleIds))
	result.Pushed += int64(subscriber.FolderSyncedCount)
	if subscriber.MessageId != nil {
		totals.messages++
	}
	if len(subscriber.SelectedArticleIds) > 0 {
		totals.selectedDatabases[database] = true
	}
	if subscriber.Error != nil {
		totals.skips = append(totals.skips, *subscriber.Error)
	}
}

func checkManualExecution(config ManualWeeklyPushConfig) error {
	if config.ExecutionControl != nil {
		if err := config.ExecutionControl.Check(); err != nil {
			return err
		}
	}
	return nil
}
