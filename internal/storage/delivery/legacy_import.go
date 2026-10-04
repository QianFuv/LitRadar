package delivery

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
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
	state := input.state
	status, legacyStatus := legacyCheckpointStatus(state.Status)
	issues, err := compactLegacy(emptyMap(state.Snapshot.IssueArticleCounts))
	if err != nil {
		return 0, 0, err
	}
	inpress, err := compactLegacy(emptyMap(state.Snapshot.InpressArticleCounts))
	if err != nil {
		return 0, 0, err
	}
	snapshot := `{"issue_article_counts":` + issues + `,"inpress_article_counts":` + inpress + `}`
	_, err = connection.ExecContext(ctx, `INSERT INTO delivery_checkpoints(workflow,db_name,status,legacy_status,snapshot_json,last_completed_run_at,revision,legacy_source_hash,legacy_source_name,legacy_imported_at,created_at,updated_at) VALUES(?,?,?,?,?,?,0,?,?,?,?,?)`, input.workflow, state.DbName, status, legacyStatus, snapshot, state.LastCompletedRunAt, input.sourceHash, input.sourceName, now, now, now)
	if err != nil {
		return 0, 0, err
	}
	var runId *int64
	itemCount := 0
	if run := state.Run; run != nil {
		status, legacyStatus := legacyRunStatus(run.Status)
		summary, err := compactLegacy(map[string]any{"pending_issue_keys": emptySlice(run.PendingIssueKeys), "done_issue_keys": emptySlice(run.DoneIssueKeys), "pending_inpress_keys": emptySlice(run.PendingInpressKeys), "done_inpress_keys": emptySlice(run.DoneInpressKeys), "delivered_article_ids": emptySlice(run.DeliveredArticleIds), "subscriber_count": len(run.UserResults)})
		if err != nil {
			return 0, 0, err
		}
		if err := validateJson(summary); err != nil {
			return 0, 0, err
		}
		inserted, err := connection.ExecContext(ctx, `INSERT INTO delivery_runs(external_id,workflow,scope_key,db_name,trigger_kind,mode,user_id,status,legacy_status,owner_id,lease_expires_at,deadline_at,cancellation_requested,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,'legacy','execute',NULL,?,?,NULL,NULL,NULL,0,?,NULL,0,?,NULL,?,?)`, run.RunId, input.workflow, state.DbName, state.DbName, status, legacyStatus, summary, now, now, now)
		if err != nil {
			return 0, 0, err
		}
		id, err := inserted.LastInsertId()
		if err != nil {
			return 0, 0, err
		}
		runId = &id
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
					return 0, 0, &Error{Kind: "invalid_legacy"}
				}
				identities[identity] = true
				if err := insertLegacyItem(ctx, connection, id, group.kind, key, nil, group.status, nil, now); err != nil {
					return 0, 0, err
				}
				itemCount++
			}
		}
		for _, result := range run.UserResults {
			userId, err := legacyNumeric(result.SubscriberId)
			if err != nil {
				return 0, 0, err
			}
			status, legacyStatus := legacyItemStatus(result.Status)
			payload, err := compactLegacy(map[string]any{"selected_count": result.SelectedCount, "pushed_count": result.PushedCount, "folder_synced_count": result.FolderSyncedCount})
			if err != nil {
				return 0, 0, err
			}
			if err := insertLegacyItem(ctx, connection, id, ItemKindSubscriber, result.SubscriberId, &userId, status, legacyStatus, now); err != nil {
				return 0, 0, err
			}
			if _, err := connection.ExecContext(ctx, `UPDATE delivery_run_items SET result_json=? WHERE delivery_run_id=? AND item_kind='subscriber' AND item_key=?`, payload, id, result.SubscriberId); err != nil {
				return 0, 0, err
			}
			itemCount++
		}
	}
	for _, key := range sortedKeys(state.DeliveryDedupe) {
		userId, articleId, err := legacyPair(key)
		if err != nil {
			return 0, 0, err
		}
		if _, err := connection.ExecContext(ctx, `INSERT INTO delivery_dedupe(workflow,db_name,user_id,article_id,delivery_run_id,status,message_id,reservation_owner,legacy_delivered_at,revision,reserved_at,delivered_at,updated_at) VALUES(?,?,?,?,?,'confirmed',NULL,NULL,?,0,?,?,?)`, input.workflow, state.DbName, userId, articleId, runId, state.DeliveryDedupe[key], now, now, now); err != nil {
			return 0, 0, err
		}
	}
	return itemCount, len(state.DeliveryDedupe), nil
}

func insertLegacyItem(ctx context.Context, connection *sql.Conn, runId int64, kind ItemKind, key string, userId *int64, status ItemStatus, legacyStatus *string, now float64) error {
	var finished *float64
	if status.IsTerminal() {
		finished = &now
	}
	_, err := connection.ExecContext(ctx, `INSERT INTO delivery_run_items(delivery_run_id,item_kind,item_key,user_id,article_id,status,legacy_status,owner_id,lease_expires_at,attempt_count,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,NULL,?,?,NULL,NULL,0,NULL,NULL,0,?,NULL,?,?)`, runId, kind, key, userId, status, legacyStatus, now, now, finished)
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
