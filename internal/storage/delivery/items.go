package delivery

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ListRunItems reads every item in durable insertion order.
func (repository *Repository) ListRunItems(ctx context.Context, runId int64) ([]RunItemRecord, error) {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	rows, err := repository.database.QueryContext(ctx, "SELECT "+itemColumns+" FROM delivery_run_items WHERE delivery_run_id=? ORDER BY id", runId)
	if err != nil {
		return nil, storageError(err)
	}
	defer rows.Close()
	result := []RunItemRecord{}
	for rows.Next() {
		item, err := scanRunItemRecord(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, *item)
	}
	return result, storageError(rows.Err())
}

// InsertRunItems inserts unique new identities without changing the original terminal-run admission behavior.
func (repository *Repository) InsertRunItems(ctx context.Context, runId int64, items []RunItemCreate, now float64) ([]RunItemRecord, error) {
	return repository.insertItems(ctx, runId, items, now, false)
}

// EnsureRunItems retains prior progress and rejects an identity whose user or article changed.
func (repository *Repository) EnsureRunItems(ctx context.Context, runId int64, items []RunItemCreate, now float64) ([]RunItemRecord, error) {
	return repository.insertItems(ctx, runId, items, now, true)
}

func (repository *Repository) insertItems(ctx context.Context, runId int64, items []RunItemCreate, now float64, shouldEnsure bool) ([]RunItemRecord, error) {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if err := validateTime(now, "Delivery item creation time is invalid"); err != nil {
		return nil, err
	}
	identities := map[struct {
		kind ItemKind
		key  string
	}]bool{}
	for _, item := range items {
		if err := validateItemCreate(item); err != nil {
			return nil, err
		}
		key := struct {
			kind ItemKind
			key  string
		}{item.ItemKind, item.ItemKey}
		if identities[key] {
			return nil, invalid("Delivery run items contain duplicate identities")
		}
		identities[key] = true
	}
	records := []RunItemRecord{}
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, runId)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if shouldEnsure && run.Status.IsTerminal() {
			return ErrConflict
		}
		query := `INSERT INTO delivery_run_items(delivery_run_id,item_kind,item_key,user_id,article_id,status,legacy_status,owner_id,lease_expires_at,attempt_count,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,?,'pending',NULL,NULL,NULL,0,NULL,NULL,0,?,NULL,?,NULL)`
		if shouldEnsure {
			query += " ON CONFLICT(delivery_run_id,item_kind,item_key) DO NOTHING"
		}
		for _, item := range items {
			if _, err := connection.ExecContext(ctx, query, runId, item.ItemKind, item.ItemKey, item.UserId, item.ArticleId, now, now); err != nil {
				return err
			}
			record, err := scanRunItemRecord(connection.QueryRowContext(ctx, "SELECT "+itemColumns+" FROM delivery_run_items WHERE delivery_run_id=? AND item_kind=? AND item_key=?", runId, item.ItemKind, item.ItemKey))
			if err != nil {
				return err
			}
			if record == nil {
				return ErrNotFound
			}
			if shouldEnsure && (!sameOptionalId(record.UserId, item.UserId) || !sameOptionalId(record.ArticleId, item.ArticleId)) {
				return ErrConflict
			}
			records = append(records, *record)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return records, nil
}

func sameOptionalId(left, right *int64) bool {
	return left == nil && right == nil || left != nil && right != nil && *left == *right
}

func requireActiveRun(ctx context.Context, connection *sql.Conn, runId int64, owner string, revision int64, now float64) error {
	run, err := loadRunRecord(ctx, connection, runId)
	if err != nil {
		return err
	}
	if run == nil {
		return ErrNotFound
	}
	if run.Revision != revision || run.OwnerId == nil || *run.OwnerId != owner || !run.Status.IsActive() || run.LeaseExpiresAt == nil || *run.LeaseExpiresAt <= now {
		return ErrConflict
	}
	return nil
}

// ClaimNextItem selects only pending or expired pre-send work; sending items are never replayed.
func (repository *Repository) ClaimNextItem(ctx context.Context, runId int64, runOwner string, runRevision int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	return repository.claimItem(ctx, runId, runOwner, runRevision, nil, itemOwner, now, seconds)
}

// ClaimItem claims a specific safe item under the current unexpired run owner.
func (repository *Repository) ClaimItem(ctx context.Context, runId int64, runOwner string, runRevision, itemId int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	return repository.claimItem(ctx, runId, runOwner, runRevision, &itemId, itemOwner, now, seconds)
}

func (repository *Repository) claimItem(ctx context.Context, runId int64, runOwner string, runRevision int64, itemId *int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if itemId != nil {
		if err := validatePositiveId(*itemId, "Delivery item id is invalid"); err != nil {
			return nil, err
		}
	}
	if err := validateIdentifier(runOwner, "Delivery run owner id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(itemOwner, "Delivery item owner id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRevisionLease(runRevision, now, seconds); err != nil {
		return nil, err
	}
	var record *RunItemRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := requireActiveRun(ctx, connection, runId, runOwner, runRevision, now); err != nil {
			return err
		}
		var id int64
		if itemId == nil {
			var selected sqlite.Integer
			err := connection.QueryRowContext(ctx, `SELECT id FROM delivery_run_items WHERE delivery_run_id=? AND (status='pending' OR (status='claimed' AND lease_expires_at<=?)) ORDER BY id LIMIT 1`, runId, now).Scan(&selected)
			if errors.Is(err, sql.ErrNoRows) {
				return nil
			}
			if err != nil {
				return err
			}
			id = int64(selected)
			if err := executeCas(ctx, connection, `UPDATE delivery_run_items SET status='claimed',owner_id=?1,lease_expires_at=?2,attempt_count=attempt_count+1,started_at=COALESCE(started_at,?3),updated_at=?4,revision=revision+1 WHERE id=?5 AND (status='pending' OR (status='claimed' AND lease_expires_at<=?3))`, itemOwner, now+seconds, now, now, id); err != nil {
				return err
			}
			record, err = requireRecord(loadRunItemRecord(ctx, connection, id))
			return err
		} else {
			id = *itemId
		}
		current, err := loadRunItemRecord(ctx, connection, id)
		if err != nil {
			return err
		}
		if current == nil {
			return ErrNotFound
		}
		if current.DeliveryRunId != runId || !(current.Status == ItemStatusPending || current.Status == ItemStatusClaimed && current.LeaseExpiresAt != nil && *current.LeaseExpiresAt <= now) {
			return ErrConflict
		}
		if err := executeCas(ctx, connection, `UPDATE delivery_run_items SET status='claimed',owner_id=?1,lease_expires_at=?2,attempt_count=attempt_count+1,started_at=COALESCE(started_at,?3),updated_at=?4,revision=revision+1 WHERE id=?5 AND delivery_run_id=?6 AND revision=?7 AND (status='pending' OR (status='claimed' AND lease_expires_at<=?3))`, itemOwner, now+seconds, now, now, id, runId, current.Revision); err != nil {
			return err
		}
		record, err = requireRecord(loadRunItemRecord(ctx, connection, id))
		return err
	})
	return record, err
}

func validateItemOwner(id int64, owner string, revision int64, now float64) error {
	if err := validatePositiveId(id, "Delivery item id is invalid"); err != nil {
		return err
	}
	if err := validateIdentifier(owner, "Delivery item owner id is invalid"); err != nil {
		return err
	}
	return validateRevisionTime(revision, now)
}

func (repository *Repository) updateItem(ctx context.Context, id int64, query string, args ...any) (*RunItemRecord, error) {
	var item *RunItemRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := executeCas(ctx, connection, query, args...); err != nil {
			return err
		}
		var err error
		item, err = requireRecord(loadRunItemRecord(ctx, connection, id))
		return err
	})
	return item, err
}

// MarkItemSending durably crosses the external side-effect boundary before any request is sent.
func (repository *Repository) MarkItemSending(ctx context.Context, id int64, owner string, revision int64, now float64) (*RunItemRecord, error) {
	if err := validateItemOwner(id, owner, revision, now); err != nil {
		return nil, err
	}
	return repository.updateItem(ctx, id, `UPDATE delivery_run_items SET status='sending',updated_at=?1,revision=revision+1 WHERE id=?2 AND owner_id=?3 AND revision=?4 AND status='claimed' AND lease_expires_at>?1`, now, id, owner, revision)
}

// FinalizeItem finalizes the exact claimed or sending owner without imposing a new expiry check.
func (repository *Repository) FinalizeItem(ctx context.Context, id int64, owner string, revision int64, status ItemStatus, result, errorCode *string, now float64) (*RunItemRecord, error) {
	if err := validateItemOwner(id, owner, revision, now); err != nil {
		return nil, err
	}
	if !status.IsTerminal() {
		return nil, invalid("Delivery item terminal status is invalid")
	}
	if err := validateOptionalJson(result); err != nil {
		return nil, err
	}
	if err := validateSymbol(errorCode, "Delivery item error code is invalid"); err != nil {
		return nil, err
	}
	return repository.updateItem(ctx, id, `UPDATE delivery_run_items SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND owner_id=? AND revision=? AND status IN ('claimed','sending')`, status, result, errorCode, now, now, id, owner, revision)
}
