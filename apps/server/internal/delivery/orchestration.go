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

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	article "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/recommend"
	"github.com/QianFuv/LitRadar/apps/server/internal/runtime/observability"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	store "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
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
	if control, ok := errors.AsType[domain.ControlError](err); ok {
		return string(control)
	}
	if failure, ok := errors.AsType[*workflowFailure](err); ok {
		return failure.kind
	}
	if _, ok := errors.AsType[*store.Error](err); ok {
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

func runRecommendationDelivery(ctx context.Context, config RunConfig, userID *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	return observeDelivery(ctx, config, userID, func(ctx context.Context) (RunOutcome, error) {
		return openAndExecuteDelivery(ctx, config, userID, manifest)
	})
}

func openAndExecuteDelivery(ctx context.Context, config RunConfig, userID *int64, manifest *recommend.ChangeManifest) (resultValue1 RunOutcome, resultError error) {
	if manifest == nil && config.ChangesFile != nil {
		loaded, err := recommend.LoadChangeManifest(*config.ChangesFile, config.DBName)
		if err != nil {
			return RunOutcome{}, classified("recommendation", err)
		}
		manifest = &loaded
	}
	repository, err := store.Open(config.AuthDBPath)
	if err != nil {
		return RunOutcome{}, classified("delivery_storage", err)
	}
	defer func() {
		if closeError := repository.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close repository: %w", closeError))
		}
	}()
	accounts, err := auth.Open(config.AuthDBPath)
	if err != nil {
		return RunOutcome{}, classified("auth_storage", err)
	}
	defer func() {
		if closeError := accounts.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close accounts: %w", closeError))
		}
	}()
	policy := settings.New(accounts, config.SecretCodec)
	timeout := time.Duration(min(max(config.TimeoutSeconds, 1), uint64((1<<63-1)/int64(time.Second)))) * time.Second
	selector := recommend.NewSelector(policy, timeout, config.RetryAttempts, config.ExecutionControl)
	sender := NewPushplusClient(config.RetryAttempts, timeout, config.ExecutionControl)
	folders := favorites.New(accounts)
	engine := deliveryEngine{repository: repository, settings: policy, selectArticles: selector.SelectForSubscriber, send: sender.Send, writeFavorites: func(ctx context.Context, writes []FavoriteWritePlan) error {
		return executeFavoriteWrites(ctx, folders, writes)
	}}
	return engine.executeInner(ctx, config, userID, manifest)
}

func (engine *deliveryEngine) execute(ctx context.Context, config RunConfig, userID *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	return observeDelivery(ctx, config, userID, func(ctx context.Context) (RunOutcome, error) {
		return engine.executeInner(ctx, config, userID, manifest)
	})
}

func observeDelivery(ctx context.Context, config RunConfig, userID *int64, execute func(context.Context) (RunOutcome, error)) (result RunOutcome, resultError error) {
	fields := map[string]any{"component": "delivery", "workflow": string(config.Workflow), "mode": string(config.Mode), "user_id": nil}
	if userID != nil {
		fields["user_id"] = *userID
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
			if subscriber.MessageID != nil {
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

func (engine *deliveryEngine) executeInner(ctx context.Context, config RunConfig, userID *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
	if manifest == nil && config.ChangesFile != nil {
		loaded, err := recommend.LoadChangeManifest(*config.ChangesFile, config.DBName)
		if err != nil {
			return RunOutcome{}, classified("recommendation", err)
		}
		manifest = &loaded
	}
	source := ""
	if manifest != nil && manifest.RunID != nil {
		source = *manifest.RunID
	} else {
		var random [16]byte
		if _, err := rand.Read(random[:]); err != nil {
			return RunOutcome{}, classified("auth_storage", err)
		}
		source = "run-" + hex.EncodeToString(random[:])
	}
	run, terminal, err := admitDurableRun(ctx, engine.repository, config, userID, deliveryExternalID(source, userID, config.AttemptID), unixNow())
	if err != nil {
		return RunOutcome{}, err
	}
	if terminal != nil {
		return runOutcome(config, terminal.ID, string(terminal.Status), nil, nil), nil
	}
	result, err := engine.executeOwned(ctx, config, userID, manifest, run)
	if err != nil {
		run.failBestEffort(context.WithoutCancel(ctx), engine.repository, deliveryErrorKind(err), unixNow())
	}
	return result, err
}

func deliveryExternalID(source string, userID *int64, attempt *string) string {
	if userID == nil {
		if attempt == nil {
			return source
		}
		return fmt.Sprintf("scheduled-run-%d", identity.Stable(source+":"+*attempt, "scheduled-delivery-attempt"))
	}
	value, namespace := fmt.Sprintf("%s:%d", source, *userID), "manual-delivery"
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
	return RunOutcome{config.DBName, config.Workflow, config.Mode, status, id, candidates, subscribers}
}
func resultJSON(value any) (string, error) {
	data, err := jsonvalue.EncodeJSON(value)
	if err != nil {
		return "", errors.New("delivery result serialization failed")
	}
	return string(data), nil
}
func runResultJSON(candidates, subscribers, selected, messages int) string {
	return fmt.Sprintf(`{"candidate_count":%d,"message_count":%d,"selected_count":%d,"subscriber_count":%d}`, candidates, messages, selected, subscribers)
}

func (engine *deliveryEngine) executeOwned(ctx context.Context, config RunConfig, userID *int64, manifest *recommend.ChangeManifest, run *durableRun) (RunOutcome, error) {
	if err := checkExecution(config); err != nil {
		return RunOutcome{}, err
	}
	snapshots, err := loadDeliverySnapshots(ctx, config, run)
	if err != nil {
		return RunOutcome{}, err
	}
	previous, current, completedAt := snapshots.previous, snapshots.current, snapshots.completedAt
	input, err := engine.prepareDeliveryInput(ctx, run, manifest, previous, current)
	if err != nil {
		return RunOutcome{}, err
	}
	if input.isEmpty() {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusSkipped, store.CheckpointStatusIdle, current, completedAt, new(`{"candidate_count":0,"subscriber_count":0}`), nil, unixNow())
		return runOutcome(config, run.run.ID, "idle", nil, nil), err
	}
	candidates, candidateIds, err := engine.prepareDeliveryCandidates(ctx, config, run, input)
	if err != nil {
		return RunOutcome{}, err
	}
	if len(candidates) == 0 {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusCompleted, store.CheckpointStatusCompleted, current, new(utcNowIso()), new(`{"candidate_count":0,"subscriber_count":0}`), nil, unixNow())
		return runOutcome(config, run.run.ID, "completed", nil, nil), err
	}
	subscribers, err := engine.filteredSubscribers(ctx, config, userID)
	if err != nil {
		return RunOutcome{}, classified("business_storage", err)
	}
	if len(subscribers) == 0 {
		err = run.finalizeWithCheckpoint(ctx, engine.repository, store.RunStatusSkipped, store.CheckpointStatusSkipped, previous, completedAt, new(runResultJSON(len(candidates), 0, 0, 0)), nil, unixNow())
		return runOutcome(config, run.run.ID, "skipped", candidateIds, nil), err
	}
	return engine.deliverToSubscribers(ctx, config, run, subscribers, candidates, candidateIds, snapshots)
}

func (engine *deliveryEngine) filteredSubscribers(ctx context.Context, config RunConfig, userID *int64) ([]domain.Subscriber, error) {
	subscribers, err := engine.loadDeliverySubscribers(ctx, config, userID)
	if err != nil {
		return nil, err
	}
	result := []domain.Subscriber{}
	for _, subscriber := range subscribers {
		if !recommend.IsDatabaseSelected(subscriber.SelectedDatabases, config.DBName) {
			continue
		}
		if config.Workflow == store.WorkflowNotify && subscriber.DeliveryMethod == "pushplus" && strings.TrimSpace(subscriber.PushplusToken) != "" || config.Workflow == store.WorkflowPush && subscriber.DeliveryMethod == "folder" && subscriber.TrackingFolderID != nil {
			result = append(result, subscriber)
		}
	}
	return result, nil
}

func subscriberID(subscriber domain.Subscriber) (int64, error) {
	id, err := strconv.ParseInt(subscriber.SubscriberID, 10, 64)
	if err != nil {
		return 0, errors.New("subscriber identifier is not a positive integer")
	}
	return id, nil
}

type deliverySnapshots struct {
	previous, current recommend.Snapshot
	completedAt       *string
}

func loadDeliverySnapshots(ctx context.Context, config RunConfig, run *durableRun) (deliverySnapshots, error) {
	previous := recommend.Snapshot{IssueArticleCounts: map[string]int64{}, InpressArticleCounts: map[string]int64{}}
	var completedAt *string
	if run.checkpoint != nil {
		parsed, err := recommend.ParseSnapshot([]byte(run.checkpoint.SnapshotJSON))
		if err != nil {
			return deliverySnapshots{}, err
		}
		previous = parsed
		completedAt = run.checkpoint.LastCompletedRunAt
	}
	issues, err := query.CollectIssueArticleCounts(ctx, config.IndexDBPath)
	if err != nil {
		return deliverySnapshots{}, classified("index_storage", err)
	}
	inpress, err := query.CollectInPressArticleCounts(ctx, config.IndexDBPath)
	if err != nil {
		return deliverySnapshots{}, classified("index_storage", err)
	}
	current := recommend.Snapshot{IssueArticleCounts: issues, InpressArticleCounts: inpress}
	return deliverySnapshots{previous, current, completedAt}, nil
}

func (engine *deliveryEngine) prepareDeliveryInput(ctx context.Context, run *durableRun, manifest *recommend.ChangeManifest, previous, current recommend.Snapshot) (deliveryInput, error) {
	items, err := engine.repository.ListRunItems(ctx, run.run.ID)
	if err != nil {
		return deliveryInput{}, err
	}
	hasInput := false
	for _, item := range items {
		if item.ItemKind != store.ItemKindSubscriber {
			hasInput = true
			break
		}
	}
	if run.didTakeOverCompetingRun && !hasInput {
		return deliveryInput{}, errors.New("recovered delivery run has no durable input items")
	}
	input := inputFromItems(items)
	if !hasInput {
		input = inputFromSource(manifest, previous, current)
		if _, err = engine.repository.EnsureRunItems(ctx, run.run.ID, input.items(), unixNow()); err != nil {
			return deliveryInput{}, err
		}
	}
	return input, nil
}

func (engine *deliveryEngine) prepareDeliveryCandidates(ctx context.Context, config RunConfig, run *durableRun, input deliveryInput) ([]article.ArticleCandidate, []int64, error) {
	candidates, err := loadCandidates(ctx, config.IndexDBPath, input)
	if err != nil {
		return nil, nil, classified("index_storage", err)
	}
	candidates = recommend.DeduplicateCandidates(candidates)
	candidateIds := make([]int64, 0, len(candidates))
	for _, candidate := range candidates {
		candidateIds = append(candidateIds, candidate.ArticleID)
	}
	if _, err = engine.repository.EnsureRunItems(ctx, run.run.ID, (deliveryInput{articleIds: candidateIds}).items(), unixNow()); err != nil {
		return nil, nil, err
	}
	if err = engine.finalizeProgress(ctx, run); err != nil {
		return nil, nil, err
	}
	return candidates, candidateIds, nil
}

func (engine *deliveryEngine) prepareSubscriberItems(ctx context.Context, run *durableRun, subscribers []domain.Subscriber) (map[string]store.RunItemRecord, error) {
	creates := make([]store.RunItemCreate, 0, len(subscribers))
	for _, subscriber := range subscribers {
		id, err := subscriberID(subscriber)
		if err != nil {
			return nil, err
		}
		creates = append(creates, store.RunItemCreate{ItemKind: store.ItemKindSubscriber, ItemKey: subscriber.SubscriberID, UserID: &id})
	}
	items, err := engine.repository.EnsureRunItems(ctx, run.run.ID, creates, unixNow())
	if err != nil {
		return nil, err
	}
	byKey := map[string]store.RunItemRecord{}
	for _, item := range items {
		byKey[item.ItemKey] = item
	}
	return byKey, nil
}

func (engine *deliveryEngine) deliverySelectionRequest(ctx context.Context, config RunConfig, candidates []article.ArticleCandidate) (recommend.SelectionRequest, error) {
	base, err := settings.CanonicalizeBaseURL(recommend.DefaultOpenaiBaseURL)
	if err != nil {
		return recommend.SelectionRequest{}, classified("business_storage", err)
	}
	allowed, err := engine.settings.AiBaseUrls(ctx)
	if err != nil {
		return recommend.SelectionRequest{}, classified("business_storage", err)
	}
	global := recommend.GlobalConfig{AiBaseURL: base, AiAllowedBaseUrls: allowed, PushplusChannel: recommend.PushplusChannel, PushplusTemplate: "markdown"}
	defaults := recommend.Defaults{MaxCandidates: 120, AiModel: recommend.DefaultOpenaiModel, Temperature: 0.2}
	if config.MaxCandidates != nil {
		defaults.MaxCandidates = max(1, *config.MaxCandidates)
	}
	records, err := engine.repository.ListDedupe(ctx, config.Workflow, config.DBName)
	if err != nil {
		return recommend.SelectionRequest{}, err
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
		dedupe[fmt.Sprintf("%d:%d", record.UserID, record.ArticleID)] = value
	}
	request := recommend.SelectionRequest{Global: global, Defaults: defaults, OverrideModel: config.AiModel, CandidatesForModel: candidates[:min(len(candidates), defaults.MaxCandidates)], CandidatesByID: recommend.CandidatesByID(candidates), Dedupe: dedupe}
	return request, nil
}

func (engine *deliveryEngine) executeSubscriberItem(ctx context.Context, config RunConfig, run *durableRun, subscriber domain.Subscriber, item store.RunItemRecord, request recommend.SelectionRequest) (SubscriberPlan, error) {
	var plan SubscriberPlan
	var err error
	if item.Status.IsTerminal() {
		plan, err = engine.terminalPlan(ctx, config, subscriber, item)
	} else {
		claimed, claimError := engine.repository.ClaimItem(ctx, run.run.ID, run.owner, run.run.Revision, item.ID, run.owner, unixNow(), deliveryLeaseSeconds)
		if claimError != nil {
			return SubscriberPlan{}, claimError
		}
		request.Subscriber = subscriber
		plan, err = engine.processSubscriber(ctx, config, run, *claimed, request)
	}
	return plan, err
}

func (engine *deliveryEngine) executeSubscriberPlans(ctx context.Context, config RunConfig, run *durableRun, subscribers []domain.Subscriber, byKey map[string]store.RunItemRecord, request recommend.SelectionRequest) ([]SubscriberPlan, error) {
	plans := []SubscriberPlan{}
	for _, subscriber := range subscribers {
		if err := checkExecution(config); err != nil {
			return nil, err
		}
		if err := run.renew(ctx, engine.repository, unixNow()); err != nil {
			return nil, err
		}
		item, exists := byKey[subscriber.SubscriberID]
		if !exists {
			return nil, errors.New("subscriber item is unavailable")
		}
		plan, err := engine.executeSubscriberItem(ctx, config, run, subscriber, item, request)
		if err != nil {
			return nil, err
		}
		plans = append(plans, plan)
	}
	return plans, nil
}

func (engine *deliveryEngine) finalizeSubscriberPlans(ctx context.Context, config RunConfig, run *durableRun, snapshots deliverySnapshots, candidateIds []int64, plans []SubscriberPlan) (RunOutcome, error) {
	previous, current, completedAt := snapshots.previous, snapshots.current, snapshots.completedAt
	status, checkpointStatus, snapshot := store.RunStatusCompleted, store.CheckpointStatusCompleted, current
	var errorCode *string
	selected, messages := 0, 0
	for _, plan := range plans {
		selected += len(plan.SelectedArticleIds)
		if plan.MessageID != nil {
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
		errorCode = new("ambiguous_delivery")
	case store.RunStatusFailed:
		checkpointStatus = store.CheckpointStatusFailed
		snapshot = previous
		errorCode = new("subscriber_failed")
	default:
		completedAt = new(utcNowIso())
	}
	err := run.finalizeWithCheckpoint(ctx, engine.repository, status, checkpointStatus, snapshot, completedAt, new(runResultJSON(len(candidateIds), len(plans), selected, messages)), errorCode, unixNow())
	return runOutcome(config, run.run.ID, string(status), candidateIds, plans), err
}

func (engine *deliveryEngine) deliverToSubscribers(ctx context.Context, config RunConfig, run *durableRun, subscribers []domain.Subscriber, candidates []article.ArticleCandidate, candidateIds []int64, snapshots deliverySnapshots) (RunOutcome, error) {
	byKey, err := engine.prepareSubscriberItems(ctx, run, subscribers)
	if err != nil {
		return RunOutcome{}, err
	}
	request, err := engine.deliverySelectionRequest(ctx, config, candidates)
	if err != nil {
		return RunOutcome{}, err
	}
	plans, err := engine.executeSubscriberPlans(ctx, config, run, subscribers, byKey, request)
	if err != nil {
		return RunOutcome{}, err
	}
	return engine.finalizeSubscriberPlans(ctx, config, run, snapshots, candidateIds, plans)
}

func (engine *deliveryEngine) loadDeliverySubscribers(ctx context.Context, config RunConfig, userID *int64) ([]domain.Subscriber, error) {
	subscribers := []domain.Subscriber{}
	if userID != nil {
		subscriber, err := engine.repository.GetSubscriber(ctx, config.SecretCodec, *userID)
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
	return subscribers, nil
}
