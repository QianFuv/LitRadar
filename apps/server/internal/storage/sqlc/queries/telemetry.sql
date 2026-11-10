-- name: Aggregate :one
SELECT aggregate FROM hourly_load WHERE hour=? AND dimensions=?;

-- name: Store :execresult
INSERT INTO hourly_load(hour,dimensions,aggregate) VALUES(?,?,?) ON CONFLICT(hour,dimensions) DO UPDATE SET aggregate=excluded.aggregate;

-- name: Prune :execresult
DELETE FROM hourly_load WHERE hour<?;

-- name: InRange :many
SELECT hour,dimensions,aggregate FROM hourly_load WHERE hour>=? AND hour<?;
