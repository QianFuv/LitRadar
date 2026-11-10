-- name: ActiveBatch :one
SELECT batch_id,status,fingerprint,selection_kind,sync_mode,issue_batch_size,notify,notify_dry_run,started_at FROM index_batches WHERE status IN ('active','abandoning');

-- name: HasPendingManifest :one
SELECT EXISTS(SELECT 1 FROM index_batch_catalogs WHERE batch_id=?1 AND phase!='completed' AND outcome_manifest_path IS NOT NULL);

-- name: AbandonBatch :execresult
UPDATE index_batches SET status='abandoning',updated_at=?2 WHERE batch_id=?1 AND status='active';

-- name: CompleteAbandonment :execresult
UPDATE index_batches SET status='abandoned',updated_at=?2,completed_at=?2 WHERE batch_id=?1 AND status='abandoning';

-- name: ReplaceBatchLease :execresult
UPDATE index_batch_lease SET batch_id=?1,heartbeat_at=?3,expires_at=?4 WHERE lease_key=1 AND batch_id=?2 AND owner_id=?5;

-- name: InsertBatch :execresult
INSERT INTO index_batches(batch_id,status,fingerprint,selection_kind,sync_mode,issue_batch_size,notify,notify_dry_run,started_at,updated_at) VALUES(?1,'active',?2,?3,?4,?5,?6,?7,?8,?8);

-- name: InsertBatchCatalog :execresult
INSERT INTO index_batch_catalogs(batch_id,ordinal,file_name,catalog_name,csv_sha256,provider_name,journal_count,phase,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8);

-- name: BatchLease :one
SELECT owner_id,expires_at FROM index_batch_lease WHERE lease_key=1;

-- name: UpsertBatchLease :execresult
INSERT INTO index_batch_lease(lease_key,batch_id,owner_id,heartbeat_at,expires_at) VALUES(1,?1,?2,?3,?4) ON CONFLICT(lease_key) DO UPDATE SET batch_id=excluded.batch_id,owner_id=excluded.owner_id,heartbeat_at=excluded.heartbeat_at,expires_at=excluded.expires_at;

-- name: OwnsBatchLease :one
SELECT EXISTS(SELECT 1 FROM index_batch_lease WHERE lease_key=1 AND batch_id=?1 AND owner_id=?2 AND expires_at>?3);

-- name: HeartbeatBatchLease :execresult
UPDATE index_batch_lease SET heartbeat_at=?3,expires_at=?4 WHERE lease_key=1 AND batch_id=?1 AND owner_id=?2 AND expires_at>?3;

-- name: ReleaseBatchLease :execresult
DELETE FROM index_batch_lease WHERE lease_key=1 AND batch_id=?1 AND owner_id=?2;

-- name: BatchLeaseOwner :one
SELECT owner_id FROM index_batch_lease WHERE lease_key=1;

-- name: TouchBatch :execresult
UPDATE index_batches SET updated_at=?2 WHERE batch_id=?1;

-- name: BatchCatalogDescriptors :many
SELECT file_name,catalog_name,csv_sha256,provider_name FROM index_batch_catalogs WHERE batch_id=?1 ORDER BY ordinal;

-- name: StoreNotifyHandoff :execresult
UPDATE index_batch_catalogs SET notify_attempt_id=?3,notify_status=?4,notify_exit_code=?5,notify_unknown_acknowledged_attempt_id=?6,notify_unknown_acknowledged_at=?7,updated_at=?8 WHERE batch_id=?1 AND ordinal=?2;

-- name: TransitionCatalogPhase :execresult
UPDATE index_batch_catalogs SET phase=?3,updated_at=?4 WHERE batch_id=?1 AND ordinal=?2;

-- name: StoreManifestIntent :execresult
UPDATE index_batch_catalogs SET phase='manifest_prepared',manifest_payload=?3,manifest_sha256=?4,manifest_through_event_id=?5,manifest_path=?6,manifest_run_id=?7,manifest_generated_at=?8,updated_at=?9 WHERE batch_id=?1 AND ordinal=?2;

-- name: StoreCatalogOutcome :execresult
UPDATE index_batch_catalogs SET run_id=?3,written_article_count=?4,source_attempt_count=?5,outcome_manifest_path=?6,updated_at=?7 WHERE batch_id=?1 AND ordinal=?2 AND journal_count=?8;

-- name: CompleteCatalog :execresult
UPDATE index_batch_catalogs SET phase='completed',completed_at=COALESCE(completed_at,?3),updated_at=?3 WHERE batch_id=?1 AND ordinal=?2;

-- name: IncompleteCatalogCount :one
SELECT COUNT(*) FROM index_batch_catalogs WHERE batch_id=?1 AND phase!='completed';

-- name: CompleteBatch :execresult
UPDATE index_batches SET status='completed',updated_at=?2,completed_at=?2 WHERE batch_id=?1 AND status='active';

-- name: BatchStart :one
SELECT started_at FROM index_batches WHERE batch_id=?1;

-- name: CatalogPhase :one
SELECT phase FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2;

-- name: ManifestIntent :one
SELECT manifest_payload,manifest_sha256,manifest_through_event_id,manifest_path,manifest_run_id,manifest_generated_at FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2;

-- name: CatalogOutcome :one
SELECT run_id,journal_count,written_article_count,source_attempt_count,outcome_manifest_path FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2;

-- name: NotifyHandoff :one
SELECT notify_attempt_id,notify_status,notify_exit_code,notify_unknown_acknowledged_attempt_id,notify_unknown_acknowledged_at FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2;

-- name: BatchCatalogs :many
SELECT ordinal,file_name,catalog_name,provider_name,journal_count,phase FROM index_batch_catalogs WHERE batch_id=?1 ORDER BY ordinal;

