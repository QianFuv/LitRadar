package delivery

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
)

func (repository *Repository) importLegacy(ctx context.Context, inputs []legacyInput, now float64) (LegacyImportResult, error) {
	result := LegacyImportResult{DiscoveredCount: len(inputs)}
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		for _, input := range inputs {
			existing, err := loadCheckpointRecord(ctx, connection, input.workflow, input.state.DbName)
			if err != nil {
				return err
			}
			if existing != nil {
				if existing.LegacySourceHash != nil && *existing.LegacySourceHash == input.sourceHash {
					result.SkippedCount++
					continue
				}
				return &Error{Kind: "legacy_conflict"}
			}
			items, dedupe, err := importLegacyState(ctx, connection, input, now)
			if err != nil {
				return err
			}
			result.ImportedCount++
			result.ItemCount += items
			result.DedupeCount += dedupe
		}
		return nil
	})
	if err != nil {
		return LegacyImportResult{}, err
	}
	return result, nil
}

func compactLegacy(value any) (string, error) {
	encoded, err := json.Marshal(value)
	if err != nil {
		return "", &Error{Kind: "json", cause: err}
	}
	decoded, err := transport.ParseJson(encoded)
	if err != nil {
		return "", &Error{Kind: "json", cause: err}
	}
	encoded, err = sources.Json(decoded)
	if err != nil {
		return "", &Error{Kind: "json", cause: err}
	}
	return string(encoded), nil
}

func emptySlice[T any](value []T) []T {
	if value == nil {
		return []T{}
	}
	return value
}
func emptyMap[V any](value map[string]V) map[string]V {
	if value == nil {
		return map[string]V{}
	}
	return value
}

func importLegacyState(ctx context.Context, connection *sql.Conn, input legacyInput, now float64) (int, int, error) {
	if err := importLegacyCheckpoint(ctx, connection, input, now); err != nil {
		return 0, 0, err
	}
	state := input.state
	runId, itemCount, err := importLegacyRun(ctx, connection, input, now)
	if err != nil {
		return 0, 0, err
	}
	if err := importLegacyDedupe(ctx, connection, input, runId, now); err != nil {
		return 0, 0, err
	}
	return itemCount, len(state.DeliveryDedupe), nil
}

func insertLegacyItem(ctx context.Context, connection *sql.Conn, runId int64, kind ItemKind, key string, userId *int64, status ItemStatus, legacyStatus *string, now float64) error {
	var finished *float64
	if status.IsTerminal() {
		finished = &now
	}
	_, err := queries.ImportItem(ctx, connection, deliverydb.ImportItemParams{DeliveryRunID: sqlite.Integer(runId), ItemKind: sqlite.Text(kind), ItemKey: sqlite.Text(key), UserID: (*sqlite.Integer)(userId), Status: sqlite.Text(status), LegacyStatus: (*sqlite.Text)(legacyStatus), CreatedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now), FinishedAt: (*sqlite.Number)(finished)})
	return err
}

func legacyCheckpointStatus(value string) (CheckpointStatus, *string) {
	value = strings.TrimSpace(value)
	switch value {
	case "", "idle":
		return CheckpointStatusIdle, nil
	case "completed":
		return CheckpointStatusCompleted, nil
	case "failed":
		return CheckpointStatusFailed, nil
	case "skipped":
		return CheckpointStatusSkipped, nil
	case "unknown":
		return CheckpointStatusUnknown, nil
	case "running":
		reason := "abandoned_active"
		return CheckpointStatusUnknown, &reason
	default:
		reason := "unrecognized"
		return CheckpointStatusUnknown, &reason
	}
}

func legacyRunStatus(value string) (RunStatus, *string) {
	value = strings.TrimSpace(value)
	status := RunStatus(value)
	if status.IsTerminal() {
		return status, nil
	}
	reason := "unrecognized"
	if status.IsActive() {
		reason = "abandoned_active"
	}
	return RunStatusUnknown, &reason
}

func legacyItemStatus(value string) (ItemStatus, *string) {
	value = strings.TrimSpace(value)
	switch value {
	case "ok", "completed", "succeeded":
		return ItemStatusSucceeded, nil
	case "error", "failed":
		return ItemStatusFailed, nil
	case "skipped":
		return ItemStatusSkipped, nil
	case "cancelled":
		return ItemStatusCancelled, nil
	case "unknown":
		return ItemStatusUnknown, nil
	default:
		reason := "unrecognized"
		return ItemStatusUnknown, &reason
	}
}

func importLegacyCheckpoint(ctx context.Context, connection *sql.Conn, input legacyInput, now float64) error {
	state := input.state
	status, legacyStatus := legacyCheckpointStatus(state.Status)
	issues, err := compactLegacy(emptyMap(state.Snapshot.IssueArticleCounts))
	if err != nil {
		return err
	}
	inpress, err := compactLegacy(emptyMap(state.Snapshot.InpressArticleCounts))
	if err != nil {
		return err
	}
	snapshot := `{"issue_article_counts":` + issues + `,"inpress_article_counts":` + inpress + `}`
	_, err = queries.ImportCheckpoint(ctx, connection, deliverydb.ImportCheckpointParams{Workflow: sqlite.Text(input.workflow), DBName: sqlite.Text(state.DbName), Status: sqlite.Text(status), LegacyStatus: (*sqlite.Text)(legacyStatus), SnapshotJSON: sqlite.Text(snapshot), LastCompletedRunAt: (*sqlite.Text)(state.LastCompletedRunAt), LegacySourceHash: new(sqlite.Text(input.sourceHash)), LegacySourceName: new(sqlite.Text(input.sourceName)), LegacyImportedAt: new(sqlite.Number(now)), CreatedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now)})
	if err != nil {
		return err
	}
	return nil
}

func importLegacyRun(ctx context.Context, connection *sql.Conn, input legacyInput, now float64) (*int64, int, error) {
	state := input.state
	var runId *int64
	itemCount := 0
	if run := state.Run; run != nil {
		id, err := createLegacyRun(ctx, connection, input, *run, now)
		if err != nil {
			return nil, 0, err
		}
		runId = &id
		itemCount, err = importLegacyProgress(ctx, connection, id, *run, now)
		if err != nil {
			return nil, 0, err
		}
		subscriberCount, err := importLegacySubscribers(ctx, connection, id, *run, now)
		if err != nil {
			return nil, 0, err
		}
		itemCount += subscriberCount
	}
	return runId, itemCount, nil
}

func importLegacyDedupe(ctx context.Context, connection *sql.Conn, input legacyInput, runId *int64, now float64) error {
	state := input.state
	for _, key := range sortedKeys(state.DeliveryDedupe) {
		userId, articleId, err := legacyPair(key)
		if err != nil {
			return err
		}
		if _, err := queries.ImportDedupe(ctx, connection, deliverydb.ImportDedupeParams{Workflow: sqlite.Text(input.workflow), DBName: sqlite.Text(state.DbName), UserID: sqlite.Integer(userId), ArticleID: sqlite.Integer(articleId), DeliveryRunID: (*sqlite.Integer)(runId), LegacyDeliveredAt: new(sqlite.Text(state.DeliveryDedupe[key])), ReservedAt: sqlite.Number(now), DeliveredAt: new(sqlite.Number(now)), UpdatedAt: sqlite.Number(now)}); err != nil {
			return err
		}
	}
	return nil
}

func createLegacyRun(ctx context.Context, connection *sql.Conn, input legacyInput, run legacyRun, now float64) (int64, error) {
	state := input.state
	status, legacyStatus := legacyRunStatus(run.Status)
	summary, err := compactLegacy(map[string]any{"pending_issue_keys": emptySlice(run.PendingIssueKeys), "done_issue_keys": emptySlice(run.DoneIssueKeys), "pending_inpress_keys": emptySlice(run.PendingInpressKeys), "done_inpress_keys": emptySlice(run.DoneInpressKeys), "delivered_article_ids": emptySlice(run.DeliveredArticleIds), "subscriber_count": len(run.UserResults)})
	if err != nil {
		return 0, err
	}
	if err := validateJson(summary); err != nil {
		return 0, err
	}
	inserted, err := queries.ImportRun(ctx, connection, deliverydb.ImportRunParams{ExternalID: sqlite.Text(run.RunId), Workflow: sqlite.Text(input.workflow), ScopeKey: sqlite.Text(state.DbName), DBName: new(sqlite.Text(state.DbName)), Status: sqlite.Text(status), LegacyStatus: (*sqlite.Text)(legacyStatus), ResultJSON: new(sqlite.Text(summary)), CreatedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now), FinishedAt: new(sqlite.Number(now))})
	if err != nil {
		return 0, err
	}
	id, err := inserted.LastInsertId()
	if err != nil {
		return 0, err
	}
	return id, nil
}

func importLegacyProgress(ctx context.Context, connection *sql.Conn, id int64, run legacyRun, now float64) (int, error) {
	itemCount := 0
	identities := map[struct {
		kind ItemKind
		key  string
	}]bool{}
	for _, group := range []struct {
		kind   ItemKind
		status ItemStatus
		keys   []string
	}{{ItemKindIssue, ItemStatusPending, run.PendingIssueKeys}, {ItemKindIssue, ItemStatusSucceeded, run.DoneIssueKeys}, {ItemKindInPress, ItemStatusPending, run.PendingInpressKeys}, {ItemKindInPress, ItemStatusSucceeded, run.DoneInpressKeys}} {
		for _, key := range group.keys {
			identity := struct {
				kind ItemKind
				key  string
			}{group.kind, key}
			if identities[identity] {
				return 0, &Error{Kind: "invalid_legacy"}
			}
			identities[identity] = true
			if err := insertLegacyItem(ctx, connection, id, group.kind, key, nil, group.status, nil, now); err != nil {
				return 0, err
			}
			itemCount++
		}
	}
	return itemCount, nil
}

func importLegacySubscribers(ctx context.Context, connection *sql.Conn, id int64, run legacyRun, now float64) (int, error) {
	itemCount := 0
	for _, result := range run.UserResults {
		userId, err := legacyNumeric(result.SubscriberId)
		if err != nil {
			return 0, err
		}
		status, legacyStatus := legacyItemStatus(result.Status)
		payload, err := compactLegacy(map[string]any{"selected_count": result.SelectedCount, "pushed_count": result.PushedCount, "folder_synced_count": result.FolderSyncedCount})
		if err != nil {
			return 0, err
		}
		if err := insertLegacyItem(ctx, connection, id, ItemKindSubscriber, result.SubscriberId, &userId, status, legacyStatus, now); err != nil {
			return 0, err
		}
		if _, err := queries.ImportSubscriberResult(ctx, connection, deliverydb.ImportSubscriberResultParams{ResultJSON: new(sqlite.Text(payload)), DeliveryRunID: sqlite.Integer(id), ItemKey: sqlite.Text(result.SubscriberId)}); err != nil {
			return 0, err
		}
		itemCount++
	}
	return itemCount, nil
}
