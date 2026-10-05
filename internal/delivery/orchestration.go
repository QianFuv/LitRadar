package delivery

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"strconv"
	"strings"
	"time"

	domainauth "github.com/QianFuv/LitRadar/internal/domain/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/recommend"
	"github.com/QianFuv/LitRadar/internal/runtime/observability"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/query"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type deliveryEngine struct {
	repository     *store.Repository
	settings       *settings.Repository
	selectArticles func(context.Context, recommend.SelectionRequest) (recommend.SelectionOutcome, error)
	send           func(context.Context, PushplusMessage) (string, error)
	writeFavorites func(context.Context, []FavoriteWritePlan) error
}

type workflowFailure struct {
	kind  string
	cause error
}

func (failure *workflowFailure) Error() string { return failure.cause.Error() }
func (failure *workflowFailure) Unwrap() error { return failure.cause }
func classified(kind string, err error) error {
	if err == nil {
		return nil
	}
	return &workflowFailure{kind, err}
}
func deliveryErrorKind(err error) string {
	var control domain.ControlError
	if errors.As(err, &control) {
		return string(control)
	}
	var failure *workflowFailure
	if errors.As(err, &failure) {
		return failure.kind
	}
	var stored *store.Error
	if errors.As(err, &stored) {
		return "delivery_storage"
	}
	if errors.Is(err, ErrBusy) {
		return "busy"
	}
	return "manual_validation"
}

// RunRecommendationDelivery executes one scheduled notification or tracking run against durable state.
func RunRecommendationDelivery(ctx context.Context, config RunConfig) (RunOutcome, error) {
	return runRecommendationDelivery(ctx, config, nil, nil)
}

func runRecommendationDelivery(ctx context.Context, config RunConfig, userId *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	return observeDelivery(ctx, config, userId, func(ctx context.Context) (RunOutcome, error) {
		return openAndExecuteDelivery(ctx, config, userId, manifest)
	})
}

func openAndExecuteDelivery(ctx context.Context, config RunConfig, userId *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	if manifest == nil && config.ChangesFile != nil {
		loaded, err := recommend.LoadChangeManifest(*config.ChangesFile, config.DbName)
		if err != nil {
			return RunOutcome{}, classified("recommendation", err)
		}
		manifest = &loaded
	}
	repository, err := store.Open(config.AuthDbPath)
	if err != nil {
		return RunOutcome{}, classified("delivery_storage", err)
	}
	defer repository.Close()
	accounts, err := auth.Open(config.AuthDbPath)
	if err != nil {
		return RunOutcome{}, classified("auth_storage", err)
	}
	defer accounts.Close()
	policy := settings.New(accounts, config.SecretCodec)
	timeout := time.Duration(min(max(config.TimeoutSeconds, 1), uint64((1<<63-1)/int64(time.Second)))) * time.Second
	selector := recommend.NewSelector(policy, timeout, config.RetryAttempts, config.ExecutionControl)
	sender := NewPushplusClient(config.RetryAttempts, timeout, config.ExecutionControl)
	folders := favorites.New(accounts)
	engine := deliveryEngine{repository: repository, settings: policy, selectArticles: selector.SelectForSubscriber, send: sender.Send, writeFavorites: func(ctx context.Context, writes []FavoriteWritePlan) error {
		return executeFavoriteWrites(ctx, folders, writes)
	}}
	return engine.executeInner(ctx, config, userId, manifest)
}

func (engine *deliveryEngine) execute(ctx context.Context, config RunConfig, userId *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	return observeDelivery(ctx, config, userId, func(ctx context.Context) (RunOutcome, error) {
		return engine.executeInner(ctx, config, userId, manifest)
	})
}

func observeDelivery(ctx context.Context, config RunConfig, userId *int64, execute func(context.Context) (RunOutcome, error)) (result RunOutcome, resultError error) {
	fields := map[string]any{"component": "delivery", "workflow": string(config.Workflow), "mode": string(config.Mode), "user_id": nil}
	if userId != nil {
		fields["user_id"] = *userId
	}
	ctx = observability.StartSpan(ctx, "litradar_worker::delivery::orchestration", "delivery.workflow", fields)
	started := time.Now()
	logger := slog.Default().With("component", "delivery", "workflow", config.Workflow, "mode", config.Mode)
	logger.InfoContext(ctx, "delivery.workflow.started", "event", "delivery.workflow.started", "outcome", "started")
	defer func() {
		if resultError != nil {
			logger.WarnContext(ctx, "delivery.workflow.failed", "event", "delivery.workflow.failed", "outcome", "failure", "status", "error", "error_kind", deliveryErrorKind(resultError), "duration_ms", time.Since(started).Milliseconds())
			return
		}
		selected, messages, failed := 0, 0, 0
		var synced uint64
		for _, subscriber := range result.Subscribers {
			selected += len(subscriber.SelectedArticleIds)
			synced += subscriber.FolderSyncedCount
			if subscriber.MessageId != nil {
				messages++
			}
			if subscriber.Status == "error" {
				failed++
			}
		}
		attributes := []any{"status", result.Status, "candidate_count", len(result.CandidateArticleIds), "subscriber_count", len(result.Subscribers), "selected_count", selected, "folder_synced_count", synced, "message_count", messages, "failed_subscriber_count", failed, "duration_ms", time.Since(started).Milliseconds()}
		if result.Status == "failed" || result.Status == "unknown" {
			logger.WarnContext(ctx, "delivery.workflow.failed", append(attributes, "event", "delivery.workflow.failed", "outcome", "failure")...)
		} else {
			logger.InfoContext(ctx, "delivery.workflow.completed", append(attributes, "event", "delivery.workflow.completed", "outcome", "success")...)
		}
	}()
	return execute(ctx)
}

func (engine *deliveryEngine) executeInner(ctx context.Context, config RunConfig, userId *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	if manifest == nil && config.ChangesFile != nil {
		loaded, err := recommend.LoadChangeManifest(*config.ChangesFile, config.DbName)
		if err != nil {
			return RunOutcome{}, classified("recommendation", err)
		}
		manifest = &loaded
	}
	source := ""
	if manifest != nil && manifest.RunId != nil {
		source = *manifest.RunId
	} else {
		var random [16]byte
		if _, err := rand.Read(random[:]); err != nil {
			return RunOutcome{}, classified("auth_storage", err)
		}
		source = "run-" + hex.EncodeToString(random[:])
	}
	run, terminal, err := admitDurableRun(ctx, engine.repository, config, userId, deliveryExternalId(source, userId, config.AttemptId), unixNow())
	if err != nil {
		return RunOutcome{}, err
	}
	if terminal != nil {
		return runOutcome(config, terminal.Id, string(terminal.Status), nil, nil), nil
	}
	result, err := engine.executeOwned(ctx, config, userId, manifest, run)
	if err != nil {
		run.failBestEffort(context.WithoutCancel(ctx), engine.repository, deliveryErrorKind(err), unixNow())
	}
	return result, err
}

func deliveryExternalId(source string, userId *int64, attempt *string) string {
	if userId == nil {
		if attempt == nil {
			return source
		}
		return fmt.Sprintf("scheduled-run-%d", identity.Stable(source+":"+*attempt, "scheduled-delivery-attempt"))
	}
	value, namespace := fmt.Sprintf("%s:%d", source, *userId), "manual-delivery"
	if attempt != nil {
		value += ":" + *attempt
		namespace = "manual-delivery-attempt"
	}
	return fmt.Sprintf("user-run-%d", identity.Stable(value, namespace))
}

func checkExecution(config RunConfig) error {
	if config.ExecutionControl != nil {
		return config.ExecutionControl.Check()
	}
	return nil
}
func utcNowIso() string { return time.Now().UTC().Format("2006-01-02T15:04:05Z") }
func runOutcome(config RunConfig, id int64, status string, candidates []int64, subscribers []SubscriberPlan) RunOutcome {
	if candidates == nil {
		candidates = []int64{}
	}
	if subscribers == nil {
		subscribers = []SubscriberPlan{}
	}
	return RunOutcome{config.DbName, config.Workflow, config.Mode, status, id, candidates, subscribers}
}
func resultJson(value any) (string, error) {
	data, err := domainauth.EncodeJson(value)
	if err != nil {
		return "", errors.New("Delivery result serialization failed")
	}
	return string(data), nil
}
func runResultJson(candidates, subscribers, selected, messages int) string {
	return fmt.Sprintf(`{"candidate_count":%d,"message_count":%d,"selected_count":%d,"subscriber_count":%d}`, candidates, messages, selected, subscribers)
}

func (engine *deliveryEngine) executeOwned(ctx context.Context, config RunConfig, userId *int64, manifest *recommend.ChangeManifest, run *durableRun) (RunOutcome, error) {
	if err := checkExecution(config); err != nil {
		return RunOutcome{}, err
	}
	previous := recommend.Snapshot{IssueArticleCounts: map[string]int64{}, InpressArticleCounts: map[string]int64{}}
	var completedAt *string
	if run.checkpoint != nil {
		parsed, err := recommend.ParseSnapshot([]byte(run.checkpoint.SnapshotJson))
		if err != nil {
			return RunOutcome{}, err
		}
		previous = parsed
		completedAt = run.checkpoint.LastCompletedRunAt
	}
	issues, err := query.CollectIssueArticleCounts(ctx, config.IndexDbPath)
	if err != nil {
		return RunOutcome{}, classified("index_storage", err)
	}
	inpress, err := query.CollectInPressArticleCounts(ctx, config.IndexDbPath)
	if err != nil {
		return RunOutcome{}, classified("index_storage", err)
	}
	current := recommend.Snapshot{IssueArticleCounts: issues, InpressArticleCounts: inpress}
	items, err := engine.repository.ListRunItems(ctx, run.run.Id)
	if err != nil {
		return RunOutcome{}, err
	}
	hasInput := false
	for _, item := range items {
		if item.ItemKind != store.ItemKindSubscriber {
			hasInput = true
			break
		}
	}
	if run.didTakeOverCompetingRun && !hasInput {
		return RunOutcome{}, errors.New("Recovered delivery run has no durable input items")
	}
	input := inputFromItems(items)
	if !hasInput {
		input = inputFromSource(manifest, previous, current)
		if _, err = engine.repository.EnsureRunItems(ctx, run.run.Id, input.items(), unixNow()); err != nil {
			return RunOutcome{}, err
		}
	}
	if input.isEmpty() {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusSkipped, store.CheckpointStatusIdle, current, completedAt, pointer(`{"candidate_count":0,"subscriber_count":0}`), nil, unixNow())
		return runOutcome(config, run.run.Id, "idle", nil, nil), err
	}
	candidates, err := loadCandidates(ctx, config.IndexDbPath, input)
	if err != nil {
		return RunOutcome{}, classified("index_storage", err)
	}
	candidates = recommend.DeduplicateCandidates(candidates)
	candidateIds := make([]int64, 0, len(candidates))
	for _, candidate := range candidates {
		candidateIds = append(candidateIds, candidate.ArticleId)
	}
	if _, err = engine.repository.EnsureRunItems(ctx, run.run.Id, (deliveryInput{articleIds: candidateIds}).items(), unixNow()); err != nil {
		return RunOutcome{}, err
	}
	if err = engine.finalizeProgress(ctx, run); err != nil {
		return RunOutcome{}, err
	}
	if len(candidates) == 0 {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusCompleted, store.CheckpointStatusCompleted, current, pointer(utcNowIso()), pointer(`{"candidate_count":0,"subscriber_count":0}`), nil, unixNow())
		return runOutcome(config, run.run.Id, "completed", nil, nil), err
	}
	subscribers, err := engine.filteredSubscribers(ctx, config, userId)
	if err != nil {
		return RunOutcome{}, classified("business_storage", err)
	}
	if len(subscribers) == 0 {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusSkipped, store.CheckpointStatusSkipped, previous, completedAt, pointer(runResultJson(len(candidates), 0, 0, 0)), nil, unixNow())
		return runOutcome(config, run.run.Id, "skipped", candidateIds, nil), err
	}
	creates := make([]store.RunItemCreate, 0, len(subscribers))
	for _, subscriber := range subscribers {
		id, err := subscriberId(subscriber)
		if err != nil {
			return RunOutcome{}, err
		}
		creates = append(creates, store.RunItemCreate{ItemKind: store.ItemKindSubscriber, ItemKey: subscriber.SubscriberId, UserId: &id})
	}
	items, err = engine.repository.EnsureRunItems(ctx, run.run.Id, creates, unixNow())
	if err != nil {
		return RunOutcome{}, err
	}
	byKey := map[string]store.RunItemRecord{}
	for _, item := range items {
		byKey[item.ItemKey] = item
	}
	base, err := settings.CanonicalizeBaseUrl(recommend.DefaultOpenaiBaseUrl)
	if err != nil {
		return RunOutcome{}, classified("business_storage", err)
	}
	allowed, err := engine.settings.AiBaseUrls(ctx)
	if err != nil {
		return RunOutcome{}, classified("business_storage", err)
	}
	global := recommend.GlobalConfig{AiBaseUrl: base, AiAllowedBaseUrls: allowed, PushplusChannel: recommend.PushplusChannel, PushplusTemplate: "markdown"}
	defaults := recommend.Defaults{MaxCandidates: 120, AiModel: recommend.DefaultOpenaiModel, Temperature: 0.2}
	if config.MaxCandidates != nil {
		defaults.MaxCandidates = max(1, *config.MaxCandidates)
	}
	records, err := engine.repository.ListDedupe(ctx, config.Workflow, config.DbName)
	if err != nil {
		return RunOutcome{}, err
	}
	dedupe := map[string]string{}
	for _, record := range records {
		timestamp := record.ReservedAt
		if record.DeliveredAt != nil {
			timestamp = *record.DeliveredAt
		}
		value := strconv.FormatFloat(timestamp, 'f', -1, 64)
		if record.LegacyDeliveredAt != nil {
			value = *record.LegacyDeliveredAt
		}
		dedupe[fmt.Sprintf("%d:%d", record.UserId, record.ArticleId)] = value
	}
	request := recommend.SelectionRequest{Global: global, Defaults: defaults, OverrideModel: config.AiModel, CandidatesForModel: candidates[:min(len(candidates), defaults.MaxCandidates)], CandidatesById: recommend.CandidatesById(candidates), Dedupe: dedupe}
	plans := []SubscriberPlan{}
	for _, subscriber := range subscribers {
		if err = checkExecution(config); err != nil {
			return RunOutcome{}, err
		}
		if err = run.renew(ctx, engine.repository, unixNow()); err != nil {
			return RunOutcome{}, err
		}
		item, exists := byKey[subscriber.SubscriberId]
		if !exists {
			return RunOutcome{}, errors.New("Subscriber item is unavailable")
		}
		var plan SubscriberPlan
		if item.Status.IsTerminal() {
			plan, err = engine.terminalPlan(ctx, config, subscriber, item)
		} else {
			claimed, claimError := engine.repository.ClaimItem(ctx, run.run.Id, run.owner, run.run.Revision, item.Id, run.owner, unixNow(), deliveryLeaseSeconds)
			if claimError != nil {
				return RunOutcome{}, claimError
			}
			request.Subscriber = subscriber
			plan, err = engine.processSubscriber(ctx, config, run, *claimed, request)
		}
		if err != nil {
			return RunOutcome{}, err
		}
		plans = append(plans, plan)
	}
	status, checkpointStatus, snapshot := store.RunStatusCompleted, store.CheckpointStatusCompleted, current
	var errorCode *string
	selected, messages := 0, 0
	for _, plan := range plans {
		selected += len(plan.SelectedArticleIds)
		if plan.MessageId != nil {
			messages++
		}
		if plan.Status == "unknown" {
			status = store.RunStatusUnknown
		} else if plan.Status == "error" && status != store.RunStatusUnknown {
			status = store.RunStatusFailed
		}
	}
	switch status {
	case store.RunStatusUnknown:
		checkpointStatus = store.CheckpointStatusUnknown
		snapshot = previous
		errorCode = pointer("ambiguous_delivery")
	case store.RunStatusFailed:
		checkpointStatus = store.CheckpointStatusFailed
		snapshot = previous
		errorCode = pointer("subscriber_failed")
	default:
		completedAt = pointer(utcNowIso())
	}
	err = run.finalizeWithCheckpoint(ctx, engine.repository, status, checkpointStatus, snapshot, completedAt, pointer(runResultJson(len(candidates), len(plans), selected, messages)), errorCode, unixNow())
	return runOutcome(config, run.run.Id, string(status), candidateIds, plans), err
}

func (engine *deliveryEngine) filteredSubscribers(ctx context.Context, config RunConfig, userId *int64) ([]domain.Subscriber, error) {
	subscribers := []domain.Subscriber{}
	if userId != nil {
		subscriber, err := engine.repository.GetSubscriber(ctx, config.SecretCodec, *userId)
		if err != nil {
			return nil, err
		}
		if subscriber != nil {
			subscribers = append(subscribers, *subscriber)
		}
	} else {
		var err error
		subscribers, err = engine.repository.ListSubscribers(ctx, config.SecretCodec)
		if err != nil {
			return nil, err
		}
	}
	result := []domain.Subscriber{}
	for _, subscriber := range subscribers {
		if !recommend.IsDatabaseSelected(subscriber.SelectedDatabases, config.DbName) {
			continue
		}
		if config.Workflow == store.WorkflowNotify && subscriber.DeliveryMethod == "pushplus" && strings.TrimSpace(subscriber.PushplusToken) != "" || config.Workflow == store.WorkflowPush && subscriber.DeliveryMethod == "folder" && subscriber.TrackingFolderId != nil {
			result = append(result, subscriber)
		}
	}
	return result, nil
}

func subscriberId(subscriber domain.Subscriber) (int64, error) {
	id, err := strconv.ParseInt(subscriber.SubscriberId, 10, 64)
	if err != nil {
		return 0, errors.New("Subscriber identifier is not a positive integer")
	}
	return id, nil
}
