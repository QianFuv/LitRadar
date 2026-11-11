-- name: All :many
SELECT key,value,updated_at FROM runtime_settings;

-- name: Logging :many
SELECT key,value FROM runtime_settings WHERE key IN ('log_filter','log_format');

-- name: Value :one
SELECT value FROM runtime_settings WHERE key=?;

-- name: Store :execresult
INSERT INTO runtime_settings(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at;
