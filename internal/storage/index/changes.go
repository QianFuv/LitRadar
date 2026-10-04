package index

import (
	"context"
	"database/sql"
	"fmt"
	"slices"
	"strconv"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ContentChangeEvent describes one durable membership change independently of its source provider.
type ContentChangeEvent struct {
	EventId         int64  `json:"event_id"`
	ContentRevision string `json:"content_revision"`
	ArticleId       int64  `json:"article_id"`
	ChangeKind      string `json:"change_kind"`
	JournalId       int64  `json:"journal_id"`
	IssueId         *int64 `json:"issue_id"`
	InPress         bool   `json:"in_press"`
	CreatedAt       string `json:"created_at"`
}

// PreparedContentChangeManifest retains exact publication bytes and their inclusive outbox cursor.
type PreparedContentChangeManifest struct {
	Payload        []byte
	ThroughEventId *int64
	EventCount     uint64
}

// Format excludes publication payload bytes from diagnostic output.
func (value PreparedContentChangeManifest) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "PreparedContentChangeManifest{payload_bytes:%d through_event_id:%v event_count:%d}", len(value.Payload), value.ThroughEventId, value.EventCount)
}

// ListContentChangeEvents reads at most 10,000 typed outbox events in identifier order.
func ListContentChangeEvents(ctx context.Context, connection *sql.Conn, after int64, limit uint64) ([]ContentChangeEvent, error) {
	if limit == 0 || limit > 10000 {
		return nil, fmt.Errorf("Invalid parameter name: content change limit must be between 1 and 10000")
	}
	rows, err := connection.QueryContext(ctx, `SELECT event_id,content_revision,article_id,change_kind,journal_id,issue_id,in_press,created_at FROM article_change_events WHERE event_id>?1 ORDER BY event_id LIMIT ?2`, after, int64(limit))
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	events := []ContentChangeEvent{}
	for rows.Next() {
		var id, article, journal, inPress sqlite.Integer
		var revision, kind, created sqlite.Text
		var issue sqlite.OptionalInteger
		if err := rows.Scan(&id, &revision, &article, &kind, &journal, &issue, &inPress, &created); err != nil {
			return nil, err
		}
		events = append(events, ContentChangeEvent{int64(id), string(revision), int64(article), string(kind), int64(journal), issue.Value, inPress != 0, string(created)})
	}
	return events, rows.Err()
}

// AcknowledgeContentChangeEvents removes only events covered by an already published inclusive cursor.
func AcknowledgeContentChangeEvents(ctx context.Context, connection *sql.Conn, through int64) (uint64, error) {
	result, err := connection.ExecContext(ctx, "DELETE FROM article_change_events WHERE event_id<=?1", through)
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return uint64(count), err
}

// DiscardContentChangeEvents clears committed events after a non-update rebuild.
func DiscardContentChangeEvents(ctx context.Context, connection *sql.Conn) (uint64, error) {
	result, err := connection.ExecContext(ctx, "DELETE FROM article_change_events")
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return uint64(count), err
}

// PrepareContentChangeManifest reads pages without acknowledging events and freezes the exact bytes to publish.
func PrepareContentChangeManifest(ctx context.Context, connection *sql.Conn, database, run, generatedAt string) (PreparedContentChangeManifest, error) {
	events := []ContentChangeEvent{}
	cursor := int64(0)
	for {
		page, err := ListContentChangeEvents(ctx, connection, cursor, 1000)
		if err != nil {
			return PreparedContentChangeManifest{}, err
		}
		if len(page) == 0 {
			break
		}
		cursor = page[len(page)-1].EventId
		events = append(events, page...)
		if len(page) < 1000 {
			break
		}
	}
	payload, err := jsonBytes(manifestPayload(database, run, generatedAt, events))
	if err != nil {
		return PreparedContentChangeManifest{}, err
	}
	var through *int64
	if len(events) > 0 {
		through = ptr(events[len(events)-1].EventId)
	}
	return PreparedContentChangeManifest{append(payload, '\n'), through, uint64(len(events))}, nil
}

func manifestPayload(database, run, generatedAt string, events []ContentChangeEvent) map[string]any {
	issues := map[string]bool{}
	inPress, notifiable, added, removed := map[int64]bool{}, map[int64]bool{}, map[int64]bool{}, map[int64]bool{}
	for _, event := range events {
		if event.InPress {
			inPress[event.JournalId] = true
		} else if event.IssueId != nil {
			issues[strconv.FormatInt(event.JournalId, 10)+":"+strconv.FormatInt(*event.IssueId, 10)] = true
		}
		if event.ChangeKind == "upsert" {
			added[event.ArticleId] = true
			notifiable[event.ArticleId] = true
		} else if event.ChangeKind == "remove" {
			removed[event.ArticleId] = true
		}
	}
	issueKeys := make([]string, 0, len(issues))
	for key := range issues {
		issueKeys = append(issueKeys, key)
	}
	slices.Sort(issueKeys)
	return map[string]any{"run_id": run, "generated_at": generatedAt, "db_name": database, "changed_issue_keys": issueKeys, "changed_inpress_journal_ids": sortedIds(inPress), "notifiable_article_ids": sortedIds(notifiable), "backfill_issue_keys": []any{}, "backfill_inpress_journal_ids": []any{}, "backfill_article_ids": []any{}, "summary": map[string]any{"changed_issue_count": len(issues), "changed_inpress_count": len(inPress), "added_article_count": len(added), "removed_article_count": len(removed), "added_article_ids": sortedIds(added), "removed_article_ids": sortedIds(removed), "issues": []any{}, "inpress": []any{}, "raw_changed_issue_count": len(issues), "raw_changed_inpress_count": len(inPress), "backfill_article_count": 0, "backfill_issue_keys": []any{}, "backfill_inpress_journal_ids": []any{}}}
}

func sortedIds(values map[int64]bool) []int64 {
	ids := make([]int64, 0, len(values))
	for id := range values {
		ids = append(ids, id)
	}
	slices.Sort(ids)
	return ids
}

// WriteContentChangeManifest publishes current bytes durably before acknowledging their outbox snapshot.
func WriteContentChangeManifest(ctx context.Context, connection *sql.Conn, database, run, generatedAt, path string) (uint64, error) {
	prepared, err := PrepareContentChangeManifest(ctx, connection, database, run, generatedAt)
	if err != nil {
		return 0, err
	}
	if err := PublishContentChangeManifest(path, prepared.Payload); err != nil {
		return 0, err
	}
	if prepared.ThroughEventId != nil {
		if _, err := AcknowledgeContentChangeEvents(ctx, connection, *prepared.ThroughEventId); err != nil {
			return 0, err
		}
	}
	return prepared.EventCount, nil
}
