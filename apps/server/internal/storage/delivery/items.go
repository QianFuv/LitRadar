package delivery

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// ListRunItems reads every item in durable insertion order.
func (repository *Repository) ListRunItems(ctx context.Context, runID int64) ([]RunItemRecord, error) {
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	rows, err := queries.Items(ctx, repository.database, deliverydb.ItemsParams{DeliveryRunID: sqlite.Integer(runID)})
	if err != nil {
		return nil, storageError(err)
	}
	result := make([]RunItemRecord, 0, len(rows))
	for _, row := range rows {
		record, err := decodeRunItemRecord(row, nil)
		if err != nil {
			return nil, err
		}
		result = append(result, *record)
	}
	return result, nil
}

// InsertRunItems inserts unique new identities without changing the original terminal-run admission behavior.
func (repository *Repository) InsertRunItems(ctx context.Context, runID int64, items []RunItemCreate, now float64) ([]RunItemRecord, error) {
	return repository.insertItems(ctx, runID, items, now, false)
}

// EnsureRunItems retains prior progress and rejects an identity whose user or article changed.
func (repository *Repository) EnsureRunItems(ctx context.Context, runID int64, items []RunItemCreate, now float64) ([]RunItemRecord, error) {
	return repository.insertItems(ctx, runID, items, now, true)
}

// insertItems preserves per-item admission and readback while reusing statements within bulk transactions.
func (repository *Repository) insertItems(ctx context.Context, runID int64, items []RunItemCreate, now float64, shouldEnsure bool) ([]RunItemRecord, error) {
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
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
	err := repository.immediate(ctx, func(connection *sql.Conn) (resultError error) {
		run, err := loadRunRecord(ctx, connection, runID)
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
		var insertStatement, selectStatement *sql.Stmt
		if len(items) > 1 {
			insertStatement, err = connection.PrepareContext(ctx, query)
			if err != nil {
				return err
			}
			defer func() {
				if closeError := insertStatement.Close(); closeError != nil {
					resultError = errors.Join(resultError, fmt.Errorf("close insertStatement: %w", closeError))
				}
			}()
			selectStatement, err = connection.PrepareContext(ctx, deliverydb.ItemIdentity)
			if err != nil {
				return err
			}
			defer func() {
				if closeError := selectStatement.Close(); closeError != nil {
					resultError = errors.Join(resultError, fmt.Errorf("close selectStatement: %w", closeError))
				}
			}()
		}
		for _, item := range items {
			if insertStatement == nil {
				_, err = connection.ExecContext(ctx, query, runID, item.ItemKind, item.ItemKey, item.UserID, item.ArticleID, now, now)
			} else {
				_, err = insertStatement.ExecContext(ctx, runID, item.ItemKind, item.ItemKey, item.UserID, item.ArticleID, now, now)
			}
			if err != nil {
				return err
			}
			var row *sql.Row
			if selectStatement == nil {
				row = connection.QueryRowContext(ctx, deliverydb.ItemIdentity, runID, item.ItemKind, item.ItemKey)
			} else {
				row = selectStatement.QueryRowContext(ctx, runID, item.ItemKind, item.ItemKey)
			}
			record, err := scanPreparedItem(row)
			if err != nil {
				return err
			}
			if record == nil {
				return ErrNotFound
			}
			if shouldEnsure && (!sameOptionalID(record.UserID, item.UserID) || !sameOptionalID(record.ArticleID, item.ArticleID)) {
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

func sameOptionalID(left, right *int64) bool {
	return left == nil && right == nil || left != nil && right != nil && *left == *right
}

func requireActiveRun(ctx context.Context, connection *sql.Conn, runID int64, owner string, revision int64, now float64) error {
	run, err := loadRunRecord(ctx, connection, runID)
	if err != nil {
		return err
	}
	if run == nil {
		return ErrNotFound
	}
	if run.Revision != revision || run.OwnerID == nil || *run.OwnerID != owner || !run.Status.IsActive() || run.LeaseExpiresAt == nil || *run.LeaseExpiresAt <= now {
		return ErrConflict
	}
	return nil
}

// ClaimNextItem selects only pending or expired pre-send work; sending items are never replayed.
func (repository *Repository) ClaimNextItem(ctx context.Context, runID int64, runOwner string, runRevision int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	return repository.claimItem(ctx, runID, runOwner, runRevision, nil, itemOwner, now, seconds)
}

// ClaimItem claims a specific safe item under the current unexpired run owner.
func (repository *Repository) ClaimItem(ctx context.Context, runID int64, runOwner string, runRevision, itemID int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	return repository.claimItem(ctx, runID, runOwner, runRevision, &itemID, itemOwner, now, seconds)
}

func (repository *Repository) claimItem(ctx context.Context, runID int64, runOwner string, runRevision int64, itemID *int64, itemOwner string, now, seconds float64) (*RunItemRecord, error) {
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if itemID != nil {
		if err := validatePositiveID(*itemID, "Delivery item id is invalid"); err != nil {
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
		if err := requireActiveRun(ctx, connection, runID, runOwner, runRevision, now); err != nil {
			return err
		}
		var id int64
		if itemID == nil {
			var selected sqlite.Integer
			err := func() error {
				queryResult, err := queries.ReadyItemID(ctx, connection, deliverydb.ReadyItemIDParams{DeliveryRunID: sqlite.Integer(runID), LeaseExpiresAt: new(sqlite.Number(now))})
				if err != nil {
					return err
				}
				selected = queryResult
				return nil
			}()
			if errors.Is(err, sql.ErrNoRows) {
				return nil
			}
			if err != nil {
				return err
			}
			id = int64(selected)
			if err := checkCAS(queries.ClaimReadyItem(ctx, connection, deliverydb.ClaimReadyItemParams{OwnerID: new(sqlite.Text(itemOwner)), LeaseExpiresAt: new(sqlite.Number(now + seconds)), StartedAt: new(sqlite.Number(now)), UpdatedAt: sqlite.Number(now), ID: sqlite.Integer(id)})); err != nil {
				return err
			}
			record, err = requireRecord(loadRunItemRecord(ctx, connection, id))
			return err
		} else {
			id = *itemID
		}
		current, err := loadRunItemRecord(ctx, connection, id)
		if err != nil {
			return err
		}
		if current == nil {
			return ErrNotFound
		}
		if current.DeliveryRunID != runID || !(current.Status == ItemStatusPending || current.Status == ItemStatusClaimed && current.LeaseExpiresAt != nil && *current.LeaseExpiresAt <= now) {
			return ErrConflict
		}
		if err := checkCAS(queries.ClaimItem(ctx, connection, deliverydb.ClaimItemParams{OwnerID: new(sqlite.Text(itemOwner)), LeaseExpiresAt: new(sqlite.Number(now + seconds)), StartedAt: new(sqlite.Number(now)), UpdatedAt: sqlite.Number(now), ID: sqlite.Integer(id), DeliveryRunID: sqlite.Integer(runID), Revision: sqlite.Integer(current.Revision)})); err != nil {
			return err
		}
		record, err = requireRecord(loadRunItemRecord(ctx, connection, id))
		return err
	})
	return record, err
}

func validateItemOwner(id int64, owner string, revision int64, now float64) error {
	if err := validatePositiveID(id, "Delivery item id is invalid"); err != nil {
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
	if err := validateOptionalJSON(result); err != nil {
		return nil, err
	}
	if err := validateSymbol(errorCode, "Delivery item error code is invalid"); err != nil {
		return nil, err
	}
	return repository.updateItem(ctx, id, `UPDATE delivery_run_items SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND owner_id=? AND revision=? AND status IN ('claimed','sending')`, status, result, errorCode, now, now, id, owner, revision)
}
