-- name: BulkAdd :execresult
INSERT OR IGNORE INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) VALUES(?,?,?,?,?,?);

-- name: BulkRemove :execresult
DELETE FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?;

-- name: BulkMove :execresult
INSERT OR IGNORE INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) SELECT source.user_id,?,source.article_id,source.db_name,source.note,? FROM favorites AS source WHERE source.user_id=? AND source.folder_id=? AND source.article_id=? AND source.db_name=?;

-- name: TotalChanges :one
SELECT total_changes();

-- name: MovedCount :one
SELECT total_changes()-?;

-- name: Memberships :many
SELECT fav.folder_id,f.name FROM favorites fav JOIN folders f ON fav.folder_id=f.id WHERE fav.user_id=? AND fav.article_id=? AND fav.db_name=?;

-- name: FolderName :one
SELECT name FROM folders WHERE id=? AND user_id=?;

-- name: CitationReferences :many
SELECT article_id,db_name FROM favorites WHERE user_id=? AND folder_id=? ORDER BY created_at DESC,id DESC LIMIT ?;

-- name: FolderID :one
SELECT id FROM folders WHERE id=? AND user_id=?;

-- name: ClearTracking :execresult
UPDATE folders SET is_tracking=0,updated_at=? WHERE user_id=?;

-- name: CreateFolder :execresult
INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(?,?,?,?,?);

-- name: Folders :many
SELECT f.id,f.name,f.is_tracking,f.created_at,COUNT(fav.id) FROM folders f LEFT JOIN favorites fav ON fav.folder_id=f.id WHERE f.user_id=? GROUP BY f.id ORDER BY f.created_at;

-- name: RenameFolder :execresult
UPDATE folders SET name=?,updated_at=? WHERE id=? AND user_id=?;

-- name: TrackingFolder :one
SELECT id,name,is_tracking,created_at,0 FROM folders WHERE user_id=? AND is_tracking=1 LIMIT 1;

-- name: SetTrackingFolder :execresult
UPDATE folders SET is_tracking=1,updated_at=? WHERE id=? AND user_id=?;

-- name: FolderTracking :one
SELECT is_tracking FROM folders WHERE id=? AND user_id=?;

-- name: NotificationDependency :one
SELECT delivery_method,pushplus_token<>'',sync_to_tracking_folder<>0 FROM notification_settings WHERE user_id=?;

-- name: DeleteFolder :execresult
DELETE FROM folders WHERE id=? AND user_id=?;

-- name: AddFavorite :one
INSERT INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(user_id,folder_id,article_id,db_name) DO NOTHING RETURNING id,folder_id,article_id,db_name,note,created_at;

-- name: Favorite :one
SELECT id,folder_id,article_id,db_name,note,created_at FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?;

-- name: RemoveFavorite :execresult
DELETE FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?;

