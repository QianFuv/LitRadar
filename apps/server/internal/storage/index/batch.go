package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync/atomic"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/batchdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// OpenBatch opens disposable project orchestration state with version preflight before journal policy.
func OpenBatch(ctx context.Context, path string) (*Connection, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
		return nil, err
	}
	database, err := sqlite.OpenMigration(path)
	if err != nil {
		return nil, err
	}
	physical, err := database.Conn(ctx)
	if err != nil {
		database.Close()
		return nil, err
	}
	connection := &Connection{physical, database}
	if _, err = physical.ExecContext(ctx, "PRAGMA busy_timeout=30000"); err == nil {
		err = InitBatch(ctx, physical)
	}
	if err != nil {
		connection.Close()
		return nil, err
	}
	return connection, nil
}

// InitBatch atomically initializes v0 or upgrades v1 notification ambiguity to v2.
func InitBatch(ctx context.Context, connection *sql.Conn) error {
	var version sqlite.Integer
	if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return err
	}
	unsupported := func() error { return &BatchError{Kind: "unsupported_version", FoundVersion: int64(version)} }
	if version > BatchSchemaVersion {
		return unsupported()
	}
	if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;"); err != nil {
		return err
	}
	if version == BatchSchemaVersion {
		return nil
	}
	return immediate(ctx, connection, func() error {
		statement := batchCreateSql
		if version == 1 {
			statement = batchUpgradeSql
		} else if version != 0 {
			return unsupported()
		}
		if _, err := connection.ExecContext(ctx, statement); err != nil {
			return err
		}
		_, err := connection.ExecContext(ctx, "PRAGMA user_version=2")
		return err
	})
}

var batchIdSequence atomic.Uint64

// NewBatchOwnerId creates an invocation identity independent of content and provider state.
func NewBatchOwnerId() string { return uniqueBatchId("index-owner") }

// NewNotifyAttemptId creates a bounded stable identity for one notification handoff.
func NewNotifyAttemptId() string { return sha256Hex([]byte(uniqueBatchId("notify-attempt")))[:32] }
func uniqueBatchId(prefix string) string {
	return fmt.Sprintf("%s-%x-%d-%d", prefix, time.Now().UnixNano(), os.Getpid(), batchIdSequence.Add(1)-1)
}

type batchHeader struct {
	batchId, status, fingerprint, selection string
	mode                                    domain.IndexSyncMode
	issueBatchSize                          uint64
	shouldNotify, isNotifyDryRun            bool
}

func activeBatchHeader(ctx context.Context, connection *sql.Conn) (*batchHeader, error) {
	var id, status, fingerprint, selection, mode sqlite.Text
	var size, notify, dry sqlite.Integer
	err := func() error {
		queryResult, err := batchQueries.ActiveBatch(ctx, connection)
		if err != nil {
			return err
		}
		id = queryResult.BatchID
		status = queryResult.Status
		fingerprint = queryResult.Fingerprint
		selection = queryResult.SelectionKind
		mode = queryResult.SyncMode
		size = queryResult.IssueBatchSize
		notify = queryResult.Notify
		dry = queryResult.NotifyDryRun
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if status != "active" && status != "abandoning" {
		return nil, batchState("stored batch status is invalid")
	}
	if selection != "all" && selection != "explicit_file" {
		return nil, batchState("stored catalog selection is invalid")
	}
	parsed, err := parseStoredSyncMode(string(mode))
	if err != nil {
		return nil, err
	}
	count, err := storedCount(int64(size))
	if err != nil {
		return nil, err
	}
	return &batchHeader{string(id), string(status), string(fingerprint), string(selection), parsed, count, notify != 0, dry != 0}, nil
}

// AdmitBatch freezes new work or resumes compatible state under a global expiring lease.
func AdmitBatch(ctx context.Context, connection *sql.Conn, request BatchRequest, shouldResume bool, owner string, now int64) (BatchAdmission, error) {
	if err := validateIdentifier(owner, "batch owner identifier must be non-empty and bounded"); err != nil {
		return BatchAdmission{}, err
	}
	var result BatchAdmission
	err := immediate(ctx, connection, func() error {
		active, err := activeBatchHeader(ctx, connection)
		if err != nil {
			return err
		}
		id := uniqueBatchId("index-batch")
		didResume := false
		isAbandoning := false
		if active == nil {
			if err := insertBatch(ctx, connection, id, request, now); err != nil {
				return err
			}
		} else {
			id = active.batchId
			if active.status == "abandoning" {
				if shouldResume {
					return &BatchError{Kind: "abandonment_pending"}
				}
				isAbandoning = true
				didResume = true
			} else if shouldResume {
				fields, err := batchMismatches(ctx, connection, *active, request)
				if err != nil {
					return err
				}
				if len(fields) != 0 {
					return &BatchError{Kind: "compatibility", Fields: fields}
				}
				didResume = true
			} else {
				isAbandoning = true
			}
		}
		if err := claimBatchLease(ctx, connection, id, owner, now); err != nil {
			return err
		}
		if active != nil {
			if isAbandoning && active.status == "active" {
				if active.shouldNotify {
					var pending sqlite.Integer
					if err := func() error {
						queryResult, err := batchQueries.HasPendingManifest(ctx, connection, batchdb.HasPendingManifestParams{BatchID: sqlite.Text(id)})
						if err != nil {
							return err
						}
						pending = sqlite.Integer(dbtypes.Boolean(queryResult))
						return nil
					}(); err != nil {
						return err
					}
					if pending != 0 {
						return &BatchError{Kind: "published_notification_pending"}
					}
				}
				changed, err := batchQueries.AbandonBatch(ctx, connection, batchdb.AbandonBatchParams{BatchID: sqlite.Text(id), UpdatedAt: sqlite.Integer(now)})
				if err = expectBatchChange(changed, err, "active batch could not enter abandonment"); err != nil {
					return err
				}
			} else if err := touchBatch(ctx, connection, id, now); err != nil {
				return err
			}
		}
		batch, err := loadBatch(ctx, connection, id, owner, didResume)
		result = BatchAdmission{isAbandoning, batch}
		return err
	})
	return result, err
}

// ReplaceAbandoningBatch retains old history after owned checkpoint cleanup and transfers the same lease.
func ReplaceAbandoningBatch(ctx context.Context, connection *sql.Conn, previous string, request BatchRequest, owner string, now int64) (IndexBatch, error) {
	if err := validateIdentifier(owner, "batch owner identifier must be non-empty and bounded"); err != nil {
		return IndexBatch{}, err
	}
	var batch IndexBatch
	err := immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, previous, owner, now); err != nil {
			return err
		}
		changed, err := batchQueries.CompleteAbandonment(ctx, connection, batchdb.CompleteAbandonmentParams{BatchID: sqlite.Text(previous), UpdatedAt: sqlite.Integer(now)})
		if err = expectBatchChange(changed, err, "batch replacement requires an abandoning batch"); err != nil {
			return err
		}
		id := uniqueBatchId("index-batch")
		if err := insertBatch(ctx, connection, id, request, now); err != nil {
			return err
		}
		changed, err = batchQueries.ReplaceBatchLease(ctx, connection, batchdb.ReplaceBatchLeaseParams{BatchID: sqlite.Text(id), BatchID_2: sqlite.Text(previous), HeartbeatAt: sqlite.Integer(now), ExpiresAt: sqlite.Integer(batchLeaseExpiry(now)), OwnerID: sqlite.Text(owner)})
		if err = expectLeaseChange(changed, err, owner); err != nil {
			return err
		}
		batch, err = loadBatch(ctx, connection, id, owner, false)
		return err
	})
	return batch, err
}

func insertBatch(ctx context.Context, connection *sql.Conn, id string, request BatchRequest, now int64) error {
	size, err := sqliteCount(request.IssueBatchSize)
	if err != nil {
		return err
	}
	if _, err := batchQueries.InsertBatch(ctx, connection, batchdb.InsertBatchParams{BatchID: sqlite.Text(id), Fingerprint: sqlite.Text(request.Fingerprint()), SelectionKind: sqlite.Text(request.Selection), SyncMode: sqlite.Text(request.Mode), IssueBatchSize: sqlite.Integer(size), Notify: sqlite.Integer(dbtypes.Boolean(request.ShouldNotify)), NotifyDryRun: sqlite.Integer(dbtypes.Boolean(request.IsNotifyDryRun)), StartedAt: sqlite.Integer(now)}); err != nil {
		return err
	}
	for ordinal, catalog := range request.Catalogs {
		if _, err := batchQueries.InsertBatchCatalog(ctx, connection, batchdb.InsertBatchCatalogParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(ordinal), FileName: sqlite.Text(catalog.Filename), CatalogName: sqlite.Text(catalog.CatalogName), CSVSHA256: sqlite.Text(catalog.CsvSha256), ProviderName: sqlite.Text(catalog.ProviderName), JournalCount: sqlite.Integer(len(catalog.Entries)), UpdatedAt: sqlite.Integer(now)}); err != nil {
			return err
		}
	}
	return nil
}

func batchMismatches(ctx context.Context, connection *sql.Conn, active batchHeader, request BatchRequest) ([]string, error) {
	if active.fingerprint == request.Fingerprint() {
		return nil, nil
	}
	mismatch := map[string]bool{"catalog_selection": active.selection != request.Selection, "sync_mode": active.mode != request.Mode, "issue_batch_size": active.issueBatchSize != request.IssueBatchSize, "notify": active.shouldNotify != request.ShouldNotify, "notify_dry_run": active.isNotifyDryRun != request.IsNotifyDryRun}
	stored, err := readBatchCatalogDescriptors(ctx, connection, active.batchId)
	if err != nil {
		return nil, err
	}
	compareBatchCatalogs(mismatch, stored, request.Catalogs)
	var fields []string
	for _, field := range []string{"catalog_selection", "catalog_order", "catalog_content", "provider_route", "sync_mode", "issue_batch_size", "notify", "notify_dry_run"} {
		if mismatch[field] {
			fields = append(fields, field)
		}
	}
	if len(fields) == 0 {
		return nil, batchState("batch fingerprint differs without a recognized compatibility field")
	}
	return fields, nil
}

func claimBatchLease(ctx context.Context, connection *sql.Conn, id, owner string, now int64) error {
	var previous sqlite.Text
	var expires sqlite.Integer
	err := func() error {
		queryResult, err := batchQueries.BatchLease(ctx, connection)
		if err != nil {
			return err
		}
		previous = queryResult.OwnerID
		expires = queryResult.ExpiresAt
		return nil
	}()
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	if err == nil && string(previous) != owner && int64(expires) > now {
		return &BatchError{Kind: "active_lease", OwnerId: string(previous), ExpiresAt: int64(expires)}
	}
	_, err = batchQueries.UpsertBatchLease(ctx, connection, batchdb.UpsertBatchLeaseParams{BatchID: sqlite.Text(id), OwnerID: sqlite.Text(owner), HeartbeatAt: sqlite.Integer(now), ExpiresAt: sqlite.Integer(batchLeaseExpiry(now))})
	return err
}
func verifyBatchOwnership(ctx context.Context, connection *sql.Conn, id, owner string, now int64) error {
	var owns sqlite.Integer
	if err := func() error {
		queryResult, err := batchQueries.OwnsBatchLease(ctx, connection, batchdb.OwnsBatchLeaseParams{BatchID: sqlite.Text(id), OwnerID: sqlite.Text(owner), ExpiresAt: sqlite.Integer(now)})
		if err != nil {
			return err
		}
		owns = sqlite.Integer(dbtypes.Boolean(queryResult))
		return nil
	}(); err != nil {
		return err
	}
	if owns == 0 {
		return &BatchError{Kind: "ownership_lost", OwnerId: owner}
	}
	return nil
}

// HeartbeatBatchLease renews only the same owner before its previous lease expires.
func HeartbeatBatchLease(ctx context.Context, connection *sql.Conn, id, owner string, now int64) error {
	changed, err := batchQueries.HeartbeatBatchLease(ctx, connection, batchdb.HeartbeatBatchLeaseParams{BatchID: sqlite.Text(id), OwnerID: sqlite.Text(owner), HeartbeatAt: sqlite.Integer(now), ExpiresAt: sqlite.Integer(batchLeaseExpiry(now))})
	return expectLeaseChange(changed, err, owner)
}

// ReleaseBatchLease is idempotent only when no competing lease remains.
func ReleaseBatchLease(ctx context.Context, connection *sql.Conn, id, owner string) error {
	changed, err := batchQueries.ReleaseBatchLease(ctx, connection, batchdb.ReleaseBatchLeaseParams{BatchID: sqlite.Text(id), OwnerID: sqlite.Text(owner)})
	if err != nil {
		return err
	}
	count, err := changed.RowsAffected()
	if err != nil || count == 1 {
		return err
	}
	_, err = batchQueries.BatchLeaseOwner(ctx, connection)
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	if err != nil {
		return err
	}
	return &BatchError{Kind: "ownership_lost", OwnerId: owner}
}
func expectLeaseChange(result sql.Result, err error, owner string) error {
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count != 1 {
		return &BatchError{Kind: "ownership_lost", OwnerId: owner}
	}
	return nil
}
func expectBatchChange(result sql.Result, err error, reason string) error {
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count != 1 {
		return batchState(reason)
	}
	return nil
}
func touchBatch(ctx context.Context, connection *sql.Conn, id string, now int64) error {
	_, err := batchQueries.TouchBatch(ctx, connection, batchdb.TouchBatchParams{BatchID: sqlite.Text(id), UpdatedAt: sqlite.Integer(now)})
	return err
}

func parseStoredSyncMode(mode string) (domain.IndexSyncMode, error) {
	parsed := domain.IndexSyncMode(mode)
	if parsed != domain.Bootstrap && parsed != domain.Incremental && parsed != domain.FullRescan {
		return "", batchState("stored synchronization mode is invalid")
	}
	return parsed, nil
}

func readBatchCatalogDescriptors(ctx context.Context, connection *sql.Conn, id string) ([][4]string, error) {
	rows, err := batchQueries.BatchCatalogDescriptors(ctx, connection, batchdb.BatchCatalogDescriptorsParams{BatchID: sqlite.Text(id)})
	if err != nil {
		return nil, err
	}
	var stored [][4]string
	for _, row := range rows {
		stored = append(stored, [4]string{string(row.FileName), string(row.CatalogName), string(row.CSVSHA256), string(row.ProviderName)})
	}
	return stored, nil
}

func hasBatchCatalogOrder(stored [][4]string, catalogs []CatalogInput) bool {
	sameOrder := len(stored) == len(catalogs)
	if sameOrder {
		for ordinal, catalog := range catalogs {
			if stored[ordinal][0] != catalog.Filename || stored[ordinal][1] != catalog.CatalogName {
				sameOrder = false
				break
			}
		}
	}
	return sameOrder
}

func compareBatchCatalogs(mismatch map[string]bool, stored [][4]string, catalogs []CatalogInput) {
	if !hasBatchCatalogOrder(stored, catalogs) {
		mismatch["catalog_order"] = true
	} else {
		for ordinal, catalog := range catalogs {
			if stored[ordinal][2] != catalog.CsvSha256 {
				mismatch["catalog_content"] = true
			}
			if stored[ordinal][3] != catalog.ProviderName {
				mismatch["provider_route"] = true
			}
		}
	}
}
