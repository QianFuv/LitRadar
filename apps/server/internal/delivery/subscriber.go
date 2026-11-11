package delivery

import (
	"context"
	"errors"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/recommend"
	store "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
)

func emptyPlan(subscriber domain.Subscriber, status, reason string) SubscriberPlan {
	return SubscriberPlan{SubscriberID: subscriber.SubscriberID, DeliveryMethod: subscriber.DeliveryMethod, Status: status, Error: &reason, SelectedArticleIds: []int64{}, FavoriteWrites: []FavoriteWritePlan{}}
}

func favoriteWrites(config RunConfig, subscriber domain.Subscriber, ids []int64) []FavoriteWritePlan {
	writes := []FavoriteWritePlan{}
	if config.Workflow == store.WorkflowNotify && !subscriber.SyncToTrackingFolder || subscriber.TrackingFolderID == nil {
		return writes
	}
	for _, id := range ids {
		writes = append(writes, FavoriteWritePlan{subscriber.UserID, *subscriber.TrackingFolderID, id, config.DBName})
	}
	return writes
}

func executeFavoriteWrites(ctx context.Context, repository *favorites.Repository, writes []FavoriteWritePlan) error {
	type destination struct{ userID, folderID int64 }
	groups := map[destination][]favorites.Add{}
	keys := []destination{}
	for _, write := range writes {
		key := destination{write.UserID, write.FolderID}
		if _, exists := groups[key]; !exists {
			keys = append(keys, key)
		}
		groups[key] = append(groups[key], favorites.Add{Reference: favorites.Reference{ArticleID: identity.ID(write.ArticleID), DBName: write.DBName}})
	}
	slices.SortFunc(keys, func(left, right destination) int {
		if left.userID < right.userID {
			return -1
		}
		if left.userID > right.userID {
			return 1
		}
		if left.folderID < right.folderID {
			return -1
		}
		if left.folderID > right.folderID {
			return 1
		}
		return 0
	})
	for _, key := range keys {
		if _, err := repository.BulkAdd(ctx, identity.ID(key.userID), key.folderID, groups[key]); err != nil {
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
		ids = append(ids, selected.ArticleID)
	}
	writes := favoriteWrites(config, request.Subscriber, ids)
	plan := SubscriberPlan{SubscriberID: request.Subscriber.SubscriberID, DeliveryMethod: request.Subscriber.DeliveryMethod, Status: "ok", SelectedArticleIds: ids, FavoriteWrites: writes, FolderSyncedCount: uint64(len(writes))}
	if config.Workflow == store.WorkflowNotify {
		plan.MessageTitle = new(recommend.BuildMessageTitle(config.DBName, label))
		plan.MessageContent = new(recommend.BuildMarkdownContent(config.DBName, label, request.Subscriber, selection.Summary, selection.Accepted, request.CandidatesByID))
		plan.WouldSendPushplus = true
	}
	return plan, nil
}

func subscriberResultJSON(plan SubscriberPlan) string {
	value, _ := resultJSON(map[string]any{"selected_article_ids": plan.SelectedArticleIds, "folder_synced_count": plan.FolderSyncedCount, "message_id": plan.MessageID})
	return value
}

func (engine *deliveryEngine) processSubscriber(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest) (SubscriberPlan, error) {
	finish := func(plan SubscriberPlan, status store.ItemStatus, code *string) (SubscriberPlan, error) {
		_, err := engine.repository.FinalizeItem(ctx, item.ID, run.owner, item.Revision, status, new(subscriberResultJSON(plan)), code, unixNow())
		return plan, err
	}
	plan, err := engine.buildPlan(ctx, config, run.run.ExternalID, request)
	if err != nil {
		return finish(emptyPlan(request.Subscriber, "error", err.Error()), store.ItemStatusFailed, new("selection_failed"))
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
	if item.ResultJSON != nil {
		parsed, err := store.ParseSubscriberResult([]byte(*item.ResultJSON))
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

func (engine *deliveryEngine) reserveSubscriberArticles(ctx context.Context, config RunConfig, run *durableRun, userID int64, ids []int64) ([]store.DedupeResolution, bool, error) {
	reservations := []store.DedupeResolution{}
	for _, id := range ids {
		reserved, err := engine.repository.ReserveDedupe(ctx, config.Workflow, config.DBName, userID, id, run.run.ID, run.owner, unixNow())
		if err != nil {
			return nil, false, err
		}
		if reserved.Kind == "existing" {
			if _, err := engine.repository.ReleaseReservations(ctx, run.run.ID, run.owner, reservations); err != nil {
				return nil, false, err
			}
			return reservations, true, nil
		}
		reservations = append(reservations, store.DedupeResolution{ID: reserved.Record.ID, ExpectedRevision: reserved.Record.Revision})
	}
	return reservations, false, nil
}

func (engine *deliveryEngine) sendSubscriberPlan(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest, plan SubscriberPlan, reservations []store.DedupeResolution) (SubscriberPlan, bool, error) {
	var err error
	if err = checkExecution(config); err != nil {
		return SubscriberPlan{}, false, err
	}
	var sending *store.RunItemRecord
	sending, err = engine.repository.MarkItemSending(ctx, item.ID, run.owner, item.Revision, unixNow())
	if err != nil {
		return SubscriberPlan{}, false, err
	}
	if plan.MessageTitle == nil {
		return SubscriberPlan{}, false, classified("pushplus", errors.New("PushPlus title is unavailable"))
	}
	if plan.MessageContent == nil {
		return SubscriberPlan{}, false, classified("pushplus", errors.New("PushPlus content is unavailable"))
	}
	messageID, sendError := engine.send(ctx, pushplusMessage(request.Subscriber, request.Global, *plan.MessageTitle, *plan.MessageContent))
	if sendError != nil {
		_, err = engine.repository.FinalizeAttempt(ctx, sending.ID, run.owner, sending.Revision, store.ItemStatusUnknown, new(subscriberResultJSON(plan)), new("ambiguous_delivery"), run.run.ID, reservations, store.DedupeStatusUnknown, nil, unixNow())
		plan.Status = "unknown"
		plan.Error = new(sendError.Error())
		return plan, false, err
	}
	plan.MessageID = &messageID
	_, err = engine.repository.FinalizeAttempt(ctx, sending.ID, run.owner, sending.Revision, store.ItemStatusSucceeded, new(subscriberResultJSON(plan)), nil, run.run.ID, reservations, store.DedupeStatusConfirmed, &messageID, unixNow())
	if err != nil {
		return SubscriberPlan{}, false, err
	}
	return plan, true, nil
}

func (engine *deliveryEngine) executeSubscriberPlan(ctx context.Context, config RunConfig, run *durableRun, item store.RunItemRecord, request recommend.SelectionRequest, plan SubscriberPlan, finish func(SubscriberPlan, store.ItemStatus, *string) (SubscriberPlan, error)) (SubscriberPlan, error) {
	userID, err := admitSubscriberExecution(config, request.Subscriber)
	if err != nil {
		return SubscriberPlan{}, err
	}
	reservations, isExisting, err := engine.reserveSubscriberArticles(ctx, config, run, userID, plan.SelectedArticleIds)
	if err != nil {
		return SubscriberPlan{}, err
	}
	if isExisting {
		return finish(emptyPlan(request.Subscriber, "skipped", "Articles were already delivered"), store.ItemStatusSkipped, nil)
	}
	release := func() error {
		_, err := engine.repository.ReleaseReservations(ctx, run.run.ID, run.owner, reservations)
		return err
	}
	if err = engine.writeFavorites(ctx, plan.FavoriteWrites); err != nil {
		if releaseError := release(); releaseError != nil {
			return SubscriberPlan{}, releaseError
		}
		plan.Status = "error"
		plan.Error = new(err.Error())
		return finish(plan, store.ItemStatusFailed, new("favorite_write_failed"))
	}
	if config.Workflow == store.WorkflowNotify {
		var shouldAdvance bool
		plan, shouldAdvance, err = engine.sendSubscriberPlan(ctx, config, run, item, request, plan, reservations)
		if !shouldAdvance {
			return plan, err
		}
	} else {
		_, err = engine.repository.FinalizeAttempt(ctx, item.ID, run.owner, item.Revision, store.ItemStatusSucceeded, new(subscriberResultJSON(plan)), nil, run.run.ID, reservations, store.DedupeStatusConfirmed, nil, unixNow())
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
	records, err := engine.repository.ListDedupe(ctx, config.Workflow, config.DBName)
	if err != nil {
		return result, err
	}
	userID := int64(0)
	if item.UserID != nil {
		userID = *item.UserID
	}
	result.MessageID = nil
	for _, record := range records {
		if record.DeliveryRunID != nil && *record.DeliveryRunID == item.DeliveryRunID && record.UserID == userID {
			result.SelectedArticleIds = append(result.SelectedArticleIds, record.ArticleID)
			if result.MessageID == nil && record.MessageID != nil {
				result.MessageID = record.MessageID
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
		return SubscriberPlan{}, errors.New("subscriber item is not terminal")
	}
	writes := favoriteWrites(config, subscriber, result.SelectedArticleIds)
	if result.FolderSyncedCount == 0 && item.Status == store.ItemStatusUnknown {
		result.FolderSyncedCount = uint64(len(writes))
	}
	return SubscriberPlan{SubscriberID: subscriber.SubscriberID, DeliveryMethod: subscriber.DeliveryMethod, Status: status, Error: item.ErrorCode, SelectedArticleIds: result.SelectedArticleIds, MessageID: result.MessageID, FavoriteWrites: writes, FolderSyncedCount: result.FolderSyncedCount}, nil
}

func admitSubscriberExecution(config RunConfig, subscriber domain.Subscriber) (int64, error) {
	if err := checkExecution(config); err != nil {
		return 0, err
	}
	userID, err := subscriberID(subscriber)
	if err != nil {
		return 0, err
	}
	return userID, nil
}
