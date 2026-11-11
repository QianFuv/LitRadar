-- name: Notifications :many
SELECT user_id,pushplus_token,ai_api_key,ai_backup_api_key FROM notification_settings;

-- name: RuntimeSettings :many
SELECT key,value FROM runtime_settings;

-- name: CNKISessions :many
SELECT user_id,session_json FROM cnki_sessions;

-- name: UpdatePushplus :execresult
UPDATE notification_settings SET pushplus_token=? WHERE user_id=?;

-- name: UpdatePrimaryAI :execresult
UPDATE notification_settings SET ai_api_key=? WHERE user_id=?;

-- name: UpdateBackupAI :execresult
UPDATE notification_settings SET ai_backup_api_key=? WHERE user_id=?;

-- name: UpdateRuntime :execresult
UPDATE runtime_settings SET value=? WHERE key=?;

-- name: UpdateCNKI :execresult
UPDATE cnki_sessions SET session_json=? WHERE user_id=?;
