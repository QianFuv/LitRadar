-- name: Checkpoint :one
SELECT id, workflow, db_name, status, legacy_status, snapshot_json, last_completed_run_at,
     revision, legacy_source_hash, legacy_source_name, legacy_imported_at, created_at, updated_at FROM delivery_checkpoints WHERE workflow=? AND db_name=?;

-- name: CreateCheckpoint :execresult
INSERT INTO delivery_checkpoints(workflow,db_name,status,legacy_status,snapshot_json,last_completed_run_at,revision,legacy_source_hash,legacy_source_name,legacy_imported_at,created_at,updated_at) VALUES(?,?,?,NULL,?,?,0,NULL,NULL,NULL,?,?) ON CONFLICT(workflow,db_name) DO NOTHING;

-- name: UpdateCheckpoint :execresult
UPDATE delivery_checkpoints SET status=?,legacy_status=NULL,snapshot_json=?,last_completed_run_at=?,revision=revision+1,updated_at=? WHERE workflow=? AND db_name=? AND revision=?;

-- name: Dedupe :one
SELECT id, workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
     reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at FROM delivery_dedupe WHERE workflow=? AND db_name=? AND user_id=? AND article_id=?;

-- name: DedupeList :many
SELECT id, workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
     reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at FROM delivery_dedupe WHERE workflow=? AND db_name=? ORDER BY user_id,article_id;

-- name: CleanupDedupe :execresult
DELETE FROM delivery_dedupe WHERE workflow=? AND db_name=? AND status='confirmed' AND delivered_at<?;

-- name: ReserveDedupe :execresult
INSERT INTO delivery_dedupe(workflow,db_name,user_id,article_id,delivery_run_id,status,message_id,reservation_owner,legacy_delivered_at,revision,reserved_at,delivered_at,updated_at) VALUES(?,?,?,?,?,'reserved',NULL,?,NULL,0,?,NULL,?) ON CONFLICT(workflow,db_name,user_id,article_id) DO NOTHING;

-- name: ResolveDedupe :execresult
UPDATE delivery_dedupe SET status=?,message_id=?,reservation_owner=NULL,delivered_at=?,updated_at=?,revision=revision+1 WHERE id=? AND delivery_run_id=? AND reservation_owner=? AND revision=? AND status='reserved';

-- name: ReleaseReservation :execresult
DELETE FROM delivery_dedupe WHERE id=? AND delivery_run_id=? AND reservation_owner=? AND revision=? AND status='reserved';

-- name: FinalizeRun :execresult
UPDATE delivery_runs SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND workflow=? AND db_name=? AND owner_id=? AND revision=? AND status IN ('claimed','running','cancelling');

-- name: FinalizeItem :execresult
UPDATE delivery_run_items SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND delivery_run_id=? AND owner_id=? AND revision=? AND status IN ('claimed','sending');

-- name: Items :many
SELECT id, delivery_run_id, item_kind, item_key, user_id, article_id, status, legacy_status,
     owner_id, lease_expires_at, attempt_count, result_json, error_code, revision,
     created_at, started_at, updated_at, finished_at FROM delivery_run_items WHERE delivery_run_id=? ORDER BY id;

-- name: ItemIdentity :one
SELECT id, delivery_run_id, item_kind, item_key, user_id, article_id, status, legacy_status,
     owner_id, lease_expires_at, attempt_count, result_json, error_code, revision,
     created_at, started_at, updated_at, finished_at FROM delivery_run_items WHERE delivery_run_id=? AND item_kind=? AND item_key=?;

-- name: ReadyItemID :one
SELECT id FROM delivery_run_items WHERE delivery_run_id=? AND (status='pending' OR (status='claimed' AND lease_expires_at<=?)) ORDER BY id LIMIT 1;

-- name: ClaimReadyItem :execresult
UPDATE delivery_run_items SET status='claimed',owner_id=?1,lease_expires_at=?2,attempt_count=attempt_count+1,started_at=COALESCE(started_at,?3),updated_at=?4,revision=revision+1 WHERE id=?5 AND (status='pending' OR (status='claimed' AND lease_expires_at<=?3));

-- name: ClaimItem :execresult
UPDATE delivery_run_items SET status='claimed',owner_id=?1,lease_expires_at=?2,attempt_count=attempt_count+1,started_at=COALESCE(started_at,?3),updated_at=?4,revision=revision+1 WHERE id=?5 AND delivery_run_id=?6 AND revision=?7 AND (status='pending' OR (status='claimed' AND lease_expires_at<=?3));

-- name: Lease :one
SELECT id, workflow, db_name, delivery_run_id, owner_id, revision, acquired_at,
     heartbeat_at, expires_at, updated_at FROM delivery_leases WHERE workflow=? AND db_name=?;

-- name: CreateLease :execresult
INSERT INTO delivery_leases(workflow,db_name,delivery_run_id,owner_id,revision,acquired_at,heartbeat_at,expires_at,updated_at) VALUES(?,?,?,?,0,?,?,?,?);

-- name: AcquireLease :execresult
UPDATE delivery_leases SET delivery_run_id=?,owner_id=?,revision=revision+1,acquired_at=?,heartbeat_at=?,expires_at=?,updated_at=? WHERE id=? AND revision=?;

-- name: RenewLease :execresult
UPDATE delivery_leases SET heartbeat_at=?1,expires_at=?2,updated_at=?3,revision=revision+1 WHERE workflow=?4 AND db_name=?5 AND delivery_run_id=?6 AND owner_id=?7 AND revision=?8 AND expires_at>?1;

-- name: ReleaseLease :execresult
UPDATE delivery_leases SET delivery_run_id=NULL,owner_id=NULL,acquired_at=NULL,heartbeat_at=NULL,expires_at=NULL,updated_at=?,revision=revision+1 WHERE workflow=? AND db_name=? AND delivery_run_id=? AND owner_id=? AND revision=?;

-- name: ImportItem :execresult
INSERT INTO delivery_run_items(delivery_run_id,item_kind,item_key,user_id,article_id,status,legacy_status,owner_id,lease_expires_at,attempt_count,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,NULL,?,?,NULL,NULL,0,NULL,NULL,0,?,NULL,?,?);

-- name: ImportCheckpoint :execresult
INSERT INTO delivery_checkpoints(workflow,db_name,status,legacy_status,snapshot_json,last_completed_run_at,revision,legacy_source_hash,legacy_source_name,legacy_imported_at,created_at,updated_at) VALUES(?,?,?,?,?,?,0,?,?,?,?,?);

-- name: ImportDedupe :execresult
INSERT INTO delivery_dedupe(workflow,db_name,user_id,article_id,delivery_run_id,status,message_id,reservation_owner,legacy_delivered_at,revision,reserved_at,delivered_at,updated_at) VALUES(?,?,?,?,?,'confirmed',NULL,NULL,?,0,?,?,?);

-- name: ImportRun :execresult
INSERT INTO delivery_runs(external_id,workflow,scope_key,db_name,trigger_kind,mode,user_id,status,legacy_status,owner_id,lease_expires_at,deadline_at,cancellation_requested,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,'legacy','execute',NULL,?,?,NULL,NULL,NULL,0,?,NULL,0,?,NULL,?,?);

-- name: ImportSubscriberResult :execresult
UPDATE delivery_run_items SET result_json=? WHERE delivery_run_id=? AND item_kind='subscriber' AND item_key=?;

-- name: LatestManualRun :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? ORDER BY id DESC LIMIT 1;

-- name: ManualRun :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? AND external_id=? ORDER BY id DESC LIMIT 1;

-- name: AdminManualRun :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE trigger_kind='manual' AND external_id=? ORDER BY id DESC LIMIT 1;

-- name: DispatchableManualRuns :many
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE trigger_kind='manual' AND (status='queued' OR (status IN ('claimed','running','cancelling') AND lease_expires_at<=?)) ORDER BY created_at,id LIMIT ?;

-- name: Run :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE id=?;

-- name: Item :one
SELECT id, delivery_run_id, item_kind, item_key, user_id, article_id, status, legacy_status,
     owner_id, lease_expires_at, attempt_count, result_json, error_code, revision,
     created_at, started_at, updated_at, finished_at FROM delivery_run_items WHERE id=?;

-- name: DedupeByID :one
SELECT id, workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
     reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at FROM delivery_dedupe WHERE id=?;

-- name: AdmitRun :execresult
INSERT INTO delivery_runs(external_id,workflow,scope_key,db_name,trigger_kind,mode,user_id,status,legacy_status,owner_id,lease_expires_at,deadline_at,cancellation_requested,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,?,?,?,'queued',NULL,NULL,NULL,?,0,NULL,NULL,0,?,NULL,?,NULL) ON CONFLICT DO NOTHING;

-- name: RunIdentity :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE workflow=? AND scope_key=? AND external_id=?;

-- name: ActiveManualRun :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? AND status IN ('queued','claimed','running','cancelling') ORDER BY id LIMIT 1;

-- name: ActiveRun :one
SELECT id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at FROM delivery_runs WHERE id<>? AND workflow=? AND db_name=? AND status IN ('claimed','running','cancelling') ORDER BY id LIMIT 1;

-- name: ClaimRun :execresult
UPDATE delivery_runs SET status=?,owner_id=?,lease_expires_at=?,started_at=COALESCE(started_at,?),updated_at=?,revision=revision+1 WHERE id=? AND revision=?;

-- name: CancelRun :execresult
UPDATE delivery_runs SET status=?,cancellation_requested=1,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND revision=?;

-- name: StoreNotificationSettings :execresult
INSERT INTO notification_settings
(user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,pushplus_template,pushplus_topic,pushplus_channel,sync_to_tracking_folder,ai_base_url,ai_api_key,ai_model,ai_system_prompt,ai_backup_base_url,ai_backup_api_key,ai_backup_model,ai_backup_system_prompt,ai_retry_attempts,enabled,created_at,updated_at)
VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
ON CONFLICT(user_id) DO UPDATE SET
keywords=excluded.keywords,directions=excluded.directions,selected_databases=excluded.selected_databases,delivery_method=excluded.delivery_method,
pushplus_token=excluded.pushplus_token,pushplus_template=excluded.pushplus_template,pushplus_topic=excluded.pushplus_topic,pushplus_channel=excluded.pushplus_channel,
sync_to_tracking_folder=excluded.sync_to_tracking_folder,ai_base_url=excluded.ai_base_url,ai_api_key=excluded.ai_api_key,ai_model=excluded.ai_model,ai_system_prompt=excluded.ai_system_prompt,
ai_backup_base_url=excluded.ai_backup_base_url,ai_backup_api_key=excluded.ai_backup_api_key,ai_backup_model=excluded.ai_backup_model,ai_backup_system_prompt=excluded.ai_backup_system_prompt,
ai_retry_attempts=excluded.ai_retry_attempts,enabled=excluded.enabled,updated_at=excluded.updated_at;

-- name: NotificationSecrets :one
SELECT pushplus_token,ai_api_key,ai_backup_api_key FROM notification_settings WHERE user_id=?;

-- name: HasTrackingFolder :one
SELECT EXISTS(SELECT 1 FROM folders WHERE user_id=? AND is_tracking=1);

-- name: QuarantineDedupe :execresult
UPDATE delivery_dedupe SET status='unknown',reservation_owner=NULL,delivered_at=?1,updated_at=?2,revision=revision+1 WHERE delivery_dedupe.delivery_run_id=?3 AND delivery_dedupe.status='reserved' AND delivery_dedupe.user_id IN (SELECT user_id FROM delivery_run_items WHERE delivery_run_id=?3 AND item_kind='subscriber' AND status='sending' AND user_id IS NOT NULL);

-- name: QuarantineItems :execresult
UPDATE delivery_run_items SET status='unknown',owner_id=NULL,lease_expires_at=NULL,error_code='abandoned_sending',updated_at=?,finished_at=?,revision=revision+1 WHERE delivery_run_id=? AND status='sending';

-- name: ReleaseDedupe :execresult
DELETE FROM delivery_dedupe WHERE delivery_run_id=? AND status='reserved';

-- name: ResetClaimedItems :execresult
UPDATE delivery_run_items SET status='pending',owner_id=NULL,lease_expires_at=NULL,updated_at=?,revision=revision+1 WHERE delivery_run_id=? AND status='claimed';

