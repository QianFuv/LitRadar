-- name: Task :one
SELECT id,name,job_spec,legacy_command,cron,timezone,timeout_seconds,coalesce,enabled,last_run_at,last_status,created_at,updated_at FROM scheduled_tasks WHERE id=?;

-- name: Tasks :many
SELECT id,name,job_spec,legacy_command,cron,timezone,timeout_seconds,coalesce,enabled,last_run_at,last_status,created_at,updated_at FROM scheduled_tasks ORDER BY created_at DESC;

-- name: CreateTask :execresult
INSERT INTO scheduled_tasks(name,job_spec,legacy_command,cron,timezone,timeout_seconds,coalesce,enabled,last_run_at,last_status,created_at,updated_at) VALUES(?,?,NULL,?,?,?,?,?,NULL,'idle',?,?);

-- name: UpdateTask :execresult
UPDATE scheduled_tasks SET name=?,job_spec=?,legacy_command=?,cron=?,timezone=?,timeout_seconds=?,coalesce=?,enabled=?,updated_at=? WHERE id=?;

-- name: DeleteTask :execresult
DELETE FROM scheduled_tasks WHERE id=?;

-- name: RecordTaskRun :execresult
UPDATE scheduled_tasks SET last_run_at=?,last_status=?,updated_at=? WHERE id=?;

-- name: HasActiveRun :one
SELECT EXISTS(SELECT 1 FROM scheduled_task_runs WHERE task_id=? AND status IN ('claimed','running'));

-- name: CreateManualRun :execresult
INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,trigger_kind,status,worker_id,claimed_at,claim_expires_at) VALUES(?,?,?,'manual','claimed',?,?,?);

-- name: LatestScheduledSlot :one
SELECT MAX(scheduled_for) FROM scheduled_task_runs WHERE task_id=? AND trigger_kind='scheduled';

-- name: PrunePendingRuns :execresult
DELETE FROM scheduled_task_runs WHERE task_id=? AND trigger_kind='scheduled' AND status='pending' AND scheduled_for<?;

-- name: EnqueueRun :execresult
INSERT OR IGNORE INTO scheduled_task_runs(task_id,task_name,scheduled_for,status) VALUES(?,?,?,'pending');

-- name: ReadyRuns :many
SELECT run.id,run.task_id,run.scheduled_for FROM scheduled_task_runs AS run JOIN scheduled_tasks AS task ON task.id=run.task_id
 WHERE run.status='pending' AND run.trigger_kind='scheduled' AND task.enabled=1 AND task.job_spec IS NOT NULL
 AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS active WHERE active.task_id=run.task_id AND active.status IN ('claimed','running'))
 AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS earlier WHERE earlier.task_id=run.task_id AND earlier.status='pending' AND earlier.trigger_kind='scheduled' AND (earlier.scheduled_for<run.scheduled_for OR (earlier.scheduled_for=run.scheduled_for AND earlier.id<run.id)))
 ORDER BY run.scheduled_for,run.id LIMIT ?;

-- name: ClaimRun :execresult
UPDATE scheduled_task_runs SET status='claimed',worker_id=?,claimed_at=?,claim_expires_at=? WHERE scheduled_task_runs.id=? AND scheduled_task_runs.status='pending' AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS active WHERE active.task_id=? AND active.status IN ('claimed','running'));

-- name: CancelUnexecutableRun :execresult
UPDATE scheduled_task_runs SET status='error',finished_at=?,claim_expires_at=NULL,output_summary='Task is no longer executable' WHERE id=? AND worker_id=? AND status='claimed';

-- name: PruneCoalescedRuns :execresult
DELETE FROM scheduled_task_runs WHERE status='pending' AND trigger_kind='scheduled' AND task_id IN (SELECT id FROM scheduled_tasks WHERE coalesce=1) AND scheduled_for<(SELECT MAX(latest.scheduled_for) FROM scheduled_task_runs AS latest WHERE latest.task_id=scheduled_task_runs.task_id AND latest.trigger_kind='scheduled');

-- name: StartRun :execresult
UPDATE scheduled_task_runs SET status='running',started_at=?,claim_expires_at=? WHERE id=? AND worker_id=? AND status='claimed';

-- name: RenewRun :execresult
UPDATE scheduled_task_runs SET claim_expires_at=? WHERE id=? AND worker_id=? AND status='running';

-- name: FinishRun :execresult
UPDATE scheduled_task_runs SET status=?,finished_at=?,claim_expires_at=NULL,output_summary=? WHERE id=? AND worker_id=? AND status IN ('claimed','running');

-- name: RecordTaskResult :execresult
UPDATE scheduled_tasks SET last_run_at=?1,last_status=?2,updated_at=?1 WHERE id=?3;

-- name: Cursor :one
SELECT last_checked_at FROM scheduler_state WHERE id=1;

-- name: RecordCheck :execresult
UPDATE scheduler_state SET last_checked_at=CASE WHEN last_checked_at IS NULL OR last_checked_at<?1 THEN ?1 ELSE last_checked_at END WHERE id=1;

-- name: WorkerHeartbeat :execresult
INSERT INTO scheduler_workers(worker_id,started_at,heartbeat_at) VALUES(?1,?2,?2) ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at;

-- name: ServiceHeartbeat :execresult
INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES('worker',?1,?2,?2) ON CONFLICT(service,instance_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at;

-- name: PruneWorkers :execresult
DELETE FROM scheduler_workers WHERE worker_id<>? AND heartbeat_at<?;

-- name: Workers :many
SELECT worker_id,started_at,heartbeat_at FROM scheduler_workers ORDER BY heartbeat_at DESC;

-- name: RecentRuns :many
SELECT id,task_id,task_name,scheduled_for,status,worker_id,claimed_at,started_at,finished_at FROM scheduled_task_runs ORDER BY scheduled_for DESC,id DESC LIMIT ?;

-- name: ExpireRunningRuns :execresult
UPDATE scheduled_task_runs SET status='unknown',finished_at=?1,claim_expires_at=NULL WHERE status='running' AND claim_expires_at<=?1;

-- name: RecordUnknownTasks :execresult
UPDATE scheduled_tasks SET last_run_at=?1,last_status='unknown',updated_at=?1 WHERE id IN (SELECT task_id FROM scheduled_task_runs WHERE status='unknown' AND finished_at=?1);

-- name: ReleaseScheduledClaims :execresult
UPDATE scheduled_task_runs SET status='pending',worker_id=NULL,claim_expires_at=NULL,claimed_at=NULL WHERE status='claimed' AND trigger_kind='scheduled' AND claim_expires_at<=?1;

-- name: CancelManualClaims :execresult
UPDATE scheduled_task_runs SET status='cancelled',finished_at=?1,claim_expires_at=NULL WHERE status='claimed' AND trigger_kind='manual' AND claim_expires_at<=?1;

-- name: RecordCancelledTasks :execresult
UPDATE scheduled_tasks SET last_run_at=?1,last_status='cancelled',updated_at=?1 WHERE id IN (SELECT task_id FROM scheduled_task_runs WHERE trigger_kind='manual' AND status='cancelled' AND finished_at=?1);

