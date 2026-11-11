-- name: Active :many
SELECT id,title,message,priority,enabled,created_at,updated_at FROM announcements WHERE enabled=1 ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END,created_at DESC;

-- name: All :many
SELECT id,title,message,priority,enabled,created_at,updated_at FROM announcements ORDER BY created_at DESC;

-- name: ByID :one
SELECT id,title,message,priority,enabled,created_at,updated_at FROM announcements WHERE id=?;

-- name: Create :execresult
INSERT INTO announcements(title,message,priority,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?);

-- name: Update :execresult
UPDATE announcements SET title=?,message=?,priority=?,enabled=?,updated_at=? WHERE id=?;

-- name: Delete :execresult
DELETE FROM announcements WHERE id=?;
