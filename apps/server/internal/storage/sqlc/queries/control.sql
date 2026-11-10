-- name: SyncAnchor :one
SELECT committed_anchor,completed_at,completed_batch_id FROM provider_sync_anchors WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3;

-- name: RunCheckpoint :one
SELECT batch_id,run_id,sync_mode,base_anchor,traversal_checkpoint,started_at,updated_at FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3;

-- name: HasAliasSyncState :one
SELECT EXISTS(SELECT 1 FROM provider_sync_anchors WHERE provider_sync_anchors.catalog_name=?1 AND provider_sync_anchors.catalog_id=?2 UNION ALL SELECT 1 FROM provider_run_checkpoints WHERE provider_run_checkpoints.catalog_name=?1 AND provider_run_checkpoints.catalog_id=?2);

-- name: BatchAnchorCount :one
SELECT COUNT(*) FROM provider_sync_anchors WHERE catalog_name=?1 AND provider_name=?2 AND completed_batch_id=?3;

-- name: BatchRunCount :one
SELECT COUNT(*) FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND batch_id=?3;

-- name: LegacyBatchRuns :many
SELECT sync_mode,started_at FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND batch_id IS NULL ORDER BY catalog_id;

-- name: HasProviderLease :one
SELECT EXISTS(SELECT 1 FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2);

-- name: AdoptRunBatch :execresult
UPDATE provider_run_checkpoints SET batch_id=?3 WHERE catalog_name=?1 AND provider_name=?2 AND batch_id IS NULL;

-- name: AdoptAnchorBatch :execresult
UPDATE provider_sync_anchors SET completed_batch_id=?3 WHERE catalog_name=?1 AND provider_name=?2 AND completed_batch_id IS NULL AND completed_at=?4;

-- name: AbandonBatchCheckpoints :execresult
DELETE FROM provider_run_checkpoints WHERE batch_id=?1;

-- name: ProviderLease :one
SELECT run_id,expires_at FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2;

-- name: AcquireLease :execresult
INSERT INTO provider_leases(catalog_name,provider_name,run_id,heartbeat_at,expires_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(catalog_name,provider_name) DO UPDATE SET run_id=excluded.run_id,heartbeat_at=excluded.heartbeat_at,expires_at=excluded.expires_at;

-- name: HeartbeatLease :execresult
UPDATE provider_leases SET heartbeat_at=?4,expires_at=?5 WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3 AND expires_at>?4;

-- name: ReleaseLease :execresult
DELETE FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3;

-- name: ResumeRun :execresult
UPDATE provider_run_checkpoints SET run_id=?5,updated_at=?9 WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?6 AND sync_mode=?7 AND base_anchor IS ?8;

-- name: AdvanceRun :execresult
UPDATE provider_run_checkpoints SET traversal_checkpoint=?8,updated_at=?9 WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?5 AND sync_mode=?6 AND base_anchor IS ?7;

-- name: OwnsJournalRun :one
SELECT EXISTS(SELECT 1 FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?5 AND sync_mode=?6 AND base_anchor IS ?7);

-- name: CompleteRun :execresult
INSERT INTO provider_sync_anchors(catalog_name,provider_name,catalog_id,committed_anchor,completed_at,completed_batch_id) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(catalog_name,provider_name,catalog_id) DO UPDATE SET committed_anchor=excluded.committed_anchor,completed_at=excluded.completed_at,completed_batch_id=excluded.completed_batch_id;

-- name: DeleteRun :execresult
DELETE FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?5 AND sync_mode=?6 AND base_anchor IS ?7;

-- name: OwnsLiveLease :one
SELECT EXISTS(SELECT 1 FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3 AND expires_at>unixepoch());

