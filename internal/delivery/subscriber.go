package delivery

import (
	"context"
	"errors"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/recommend"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
)

func emptyPlan(subscriber domain.Subscriber, status, reason string) SubscriberPlan {
	return SubscriberPlan{SubscriberId: subscriber.SubscriberId, DeliveryMethod: subscriber.DeliveryMethod, Status: status, Error: &reason, SelectedArticleIds: []int64{}, FavoriteWrites: []FavoriteWritePlan{}}
}

func favoriteWrites(config RunConfig, subscriber domain.Subscriber, ids []int64) []FavoriteWritePlan {
	writes := []FavoriteWritePlan{}
	if config.Workflow == store.WorkflowNotify && !subscriber.SyncToTrackingFolder || subscriber.TrackingFolderId == nil {
		return writes
	}
	for _, id := range ids {
		writes = append(writes, FavoriteWritePlan{subscriber.UserId, *subscriber.TrackingFolderId, id, config.DbName})
	}
	return writes
}

func executeFavoriteWrites(ctx context.Context, repository *favorites.Repository, writes []FavoriteWritePlan) error {
	type destination struct{ userId, folderId int64 }
	groups := map[destination][]favorites.Add{}
	keys := []destination{}
	for _, write := range writes {
		key := destination{write.UserId, write.FolderId}
		if _, exists := groups[key]; !exists {
			keys = append(keys, key)
		}
		groups[key] = append(groups[key], favorites.Add{Reference: favorites.Reference{ArticleId: identity.Id(write.ArticleId), DbName: write.DbName}})
	}
	slices.SortFunc(keys, func(left, right destination) int {
		if left.userId < right.userId {
			return -1
		}
		if left.userId > right.userId {
			return 1
		}
		if left.folderId < right.folderId {
			return -1
		}
		if left.folderId > right.folderId {
			return 1
		}
		return 0
	})
	for _, key := range keys {
		if _, err := repository.BulkAdd(ctx, identity.Id(key.userId), key.folderId, groups[key]); err != nil {
			return err
		}
	}
	return nil
}

func (engine *deliveryEngine) buildPlan(ctx context.Context, config RunConfig, label string, request recommend.SelectionRequest) (SubscriberPlan, error) {
	selection, err := engine.selectArticles(ctx, request)
	if err != nil {
		return SubscriberPlan{}, err
	}
	if selection.SkipReason != nil {
		return emptyPlan(request.Subscriber, "skipped", *selection.SkipReason), nil
	}
	if len(selection.Accepted) == 0 {
		return emptyPlan(request.Subscriber, "skipped", "AI selection found no matching articles"), nil
	}
	ids := make([]int64, 0, len(selection.Accepted))
	for _, selected := range selection.Accepted {
		ids = append(ids, selected.ArticleId)
	}
	writes := favoriteWrites(config, request.Subscriber, ids)
	plan := SubscriberPlan{SubscriberId: request.Subscriber.SubscriberId, DeliveryMethod: request.Subscriber.DeliveryMethod, Status: "ok", SelectedArticleIds: ids, FavoriteWrites: writes, FolderSyncedCount: uint64(len(writes))}
	if config.Workflow == store.WorkflowNotify {
		plan.MessageTitle = pointer(recommend.BuildMessageTitle(config.DbName, label))
		plan.MessageContent = pointer(recommend.BuildMarkdownContent(config.DbName, label, request.Subscriber, selection.Summary, selection.Accepted, request.CandidatesById))
		plan.WouldSendPushplus = true
	}
	return plan, nil
}

func subscriberResultJson(plan SubscriberPlan) string {
	value, _ := resultJson(map[string]any{"selected_article_ids": plan.SelectedArticleIds, "folder_synced_count": plan.FolderSyncedCount, "message_id": plan.MessageId})
	return value
}

func (engine *deliveryEngine) processSubscriber(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest) (SubscriberPlan, error) {
	finish := func(plan SubscriberPlan, status store.ItemStatus, code *string) (SubscriberPlan, error) {
		_, err := engine.repository.FinalizeItem(ctx, item.Id, run.owner, item.Revision, status, pointer(subscriberResultJson(plan)), code, unixNow())
		return plan, err
	}
	plan, err := engine.buildPlan(ctx, config, run.run.ExternalId, request)
	if err != nil {
		return finish(emptyPlan(request.Subscriber, "error", err.Error()), store.ItemStatusFailed, pointer("selection_failed"))
	}
	if plan.Status == "skipped" {
		return finish(plan, store.ItemStatusSkipped, nil)
	}
	if config.Mode == store.RunModeDryRun {
		return finish(plan, store.ItemStatusSucceeded, nil)
	}
	return engine.executeSubscriberPlan(ctx, config, run, item, request, plan, finish)
}

func pushplusMessage(subscriber domain.Subscriber, global recommend.GlobalConfig, title, content string) PushplusMessage {
	nonempty := func(value *string, fallback string) string {
		if value != nil && strings.TrimSpace(*value) != "" {
			return *value
		}
		return fallback
	}
	topic := subscriber.Topic
	if topic == nil || strings.TrimSpace(*topic) == "" {
		topic = global.PushplusTopic
	}
	return PushplusMessage{Token: subscriber.PushplusToken, Title: title, Content: content, Channel: nonempty(subscriber.Channel, global.PushplusChannel), Template: nonempty(subscriber.Template, global.PushplusTemplate), Topic: topic, Option: global.PushplusOption}
}

func (engine *deliveryEngine) terminalPlan(ctx context.Context, config RunConfig, subscriber domain.Subscriber, item store.RunItemRecord) (SubscriberPlan, error) {
	result := store.SubscriberResult{SelectedArticleIds: []int64{}}
	if item.ResultJson != nil {
		parsed, err := store.ParseSubscriberResult([]byte(*item.ResultJson))
		if err != nil {
			return SubscriberPlan{}, err
		}
		result = parsed
	}
	if len(result.SelectedArticleIds) == 0 && item.Status == store.ItemStatusUnknown {
		result, err := engine.recoverUnknownSubscriber(ctx, config, item, result)
		if err != nil {
			return SubscriberPlan{}, err
		}
		return terminalSubscriberPlan(config, subscriber, item, result)
	}
	return terminalSubscriberPlan(config, subscriber, item, result)
}

func (engine *deliveryEngine) reserveSubscriberArticles(ctx context.Context, config RunConfig, run *durableRun, userId int64, ids []int64) ([]store.DedupeResolution, bool, error) {
	reservations := []store.DedupeResolution{}
	for _, id := range ids {
		reserved, err := engine.repository.ReserveDedupe(ctx, config.Workflow, config.DbName, userId, id, run.run.Id, run.owner, unixNow())
		if err != nil {
			return nil, false, err
		}
		if reserved.Kind == "existing" {
			if _, err := engine.repository.ReleaseReservations(ctx, run.run.Id, run.owner, reservations); err != nil {
				return nil, false, err
			}
			return reservations, true, nil
		}
		reservations = append(reservations, store.DedupeResolution{Id: reserved.Record.Id, ExpectedRevision: reserved.Record.Revision})
	}
	return reservations, false, nil
}

func (engine *deliveryEngine) sendSubscriberPlan(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest, plan SubscriberPlan, reservations []store.DedupeResolution) (SubscriberPlan, bool, error) {
	var err error
	if err = checkExecution(config); err != nil {
		return SubscriberPlan{}, false, err
	}
	var sending *store.RunItemRecord
	sending, err = engine.repository.MarkItemSending(ctx, item.Id, run.owner, item.Revision, unixNow())
	if err != nil {
		return SubscriberPlan{}, false, err
	}
	if plan.MessageTitle == nil {
		return SubscriberPlan{}, false, classified("pushplus", errors.New("PushPlus title is unavailable"))
	}
	if plan.MessageContent == nil {
		return SubscriberPlan{}, false, classified("pushplus", errors.New("PushPlus content is unavailable"))
	}
	messageId, sendError := engine.send(ctx, pushplusMessage(request.Subscriber, request.Global, *plan.MessageTitle, *plan.MessageContent))
	if sendError != nil {
		_, err = engine.repository.FinalizeAttempt(ctx, sending.Id, run.owner, sending.Revision, store.ItemStatusUnknown, pointer(subscriberResultJson(plan)), pointer("ambiguous_delivery"), run.run.Id, reservations, store.DedupeStatusUnknown, nil, unixNow())
		plan.Status = "unknown"
		plan.Error = pointer(sendError.Error())
		return plan, false, err
	}
	plan.MessageId = &messageId
	_, err = engine.repository.FinalizeAttempt(ctx, sending.Id, run.owner, sending.Revision, store.ItemStatusSucceeded, pointer(subscriberResultJson(plan)), nil, run.run.Id, reservations, store.DedupeStatusConfirmed, &messageId, unixNow())
	if err != nil {
		return SubscriberPlan{}, false, err
	}
	return plan, true, nil
}

func (engine *deliveryEngine) executeSubscriberPlan(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest, plan SubscriberPlan, finish func(SubscriberPlan, store.ItemStatus, *string) (SubscriberPlan, error)) (SubscriberPlan, error) {
	userId, err := admitSubscriberExecution(config, request.Subscriber)
	if err != nil {
		return SubscriberPlan{}, err
	}
	reservations, isExisting, err := engine.reserveSubscriberArticles(ctx, config, run, userId, plan.SelectedArticleIds)
	if err != nil {
		return SubscriberPlan{}, err
	}
	if isExisting {
		return finish(emptyPlan(request.Subscriber, "skipped", "Articles were already delivered"), store.ItemStatusSkipped, nil)
	}
	release := func() error {
		_, err := engine.repository.ReleaseReservations(ctx, run.run.Id, run.owner, reservations)
		return err
	}
	if err = engine.writeFavorites(ctx, plan.FavoriteWrites); err != nil {
		if releaseError := release(); releaseError != nil {
			return SubscriberPlan{}, releaseError
		}
		plan.Status = "error"
		plan.Error = pointer(err.Error())
		return finish(plan, store.ItemStatusFailed, pointer("favorite_write_failed"))
	}
	if config.Workflow == store.WorkflowNotify {
		var shouldAdvance bool
		plan, shouldAdvance, err = engine.sendSubscriberPlan(ctx, config, run, item, request, plan, reservations)
		if !shouldAdvance {
			return plan, err
		}
	} else {
		_, err = engine.repository.FinalizeAttempt(ctx, item.Id, run.owner, item.Revision, store.ItemStatusSucceeded, pointer(subscriberResultJson(plan)), nil, run.run.Id, reservations, store.DedupeStatusConfirmed, nil, unixNow())
	}
	if err != nil {
		return SubscriberPlan{}, err
	}
	for _, id := range plan.SelectedArticleIds {
		request.Dedupe[recommend.DeliveryKey(request.Subscriber, id)] = utcNowIso()
	}
	return plan, nil
}

func (engine *deliveryEngine) recoverUnknownSubscriber(ctx context.Context, config RunConfig, item store.RunItemRecord, result store.SubscriberResult) (store.SubscriberResult, error) {
	records, err := engine.repository.ListDedupe(ctx, config.Workflow, config.DbName)
	if err != nil {
		return result, err
	}
	userId := int64(0)
	if item.UserId != nil {
		userId = *item.UserId
	}
	result.MessageId = nil
	for _, record := range records {
		if record.DeliveryRunId != nil && *record.DeliveryRunId == item.DeliveryRunId && record.UserId == userId {
			result.SelectedArticleIds = append(result.SelectedArticleIds, record.ArticleId)
			if result.MessageId == nil && record.MessageId != nil {
				result.MessageId = record.MessageId
			}
		}
	}
	slices.Sort(result.SelectedArticleIds)
	result.SelectedArticleIds = slices.Compact(result.SelectedArticleIds)
	return result, nil
}

func terminalSubscriberPlan(config RunConfig, subscriber domain.Subscriber, item store.RunItemRecord, result store.SubscriberResult) (SubscriberPlan, error) {
	status := ""
	switch item.Status {
	case store.ItemStatusSucceeded:
		status = "ok"
	case store.ItemStatusSkipped:
		status = "skipped"
	case store.ItemStatusUnknown:
		status = "unknown"
	case store.ItemStatusFailed, store.ItemStatusCancelled:
		status = "error"
	default:
		return SubscriberPlan{}, errors.New("Subscriber item is not terminal")
	}
	writes := favoriteWrites(config, subscriber, result.SelectedArticleIds)
	if result.FolderSyncedCount == 0 && item.Status == store.ItemStatusUnknown {
		result.FolderSyncedCount = uint64(len(writes))
	}
	return SubscriberPlan{SubscriberId: subscriber.SubscriberId, DeliveryMethod: subscriber.DeliveryMethod, Status: status, Error: item.ErrorCode, SelectedArticleIds: result.SelectedArticleIds, MessageId: result.MessageId, FavoriteWrites: writes, FolderSyncedCount: result.FolderSyncedCount}, nil
}

func admitSubscriberExecution(config RunConfig, subscriber domain.Subscriber) (int64, error) {
	if err := checkExecution(config); err != nil {
		return 0, err
	}
	userId, err := subscriberId(subscriber)
	if err != nil {
		return 0, err
	}
	return userId, nil
}
