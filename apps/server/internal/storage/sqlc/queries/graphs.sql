-- name: CreateGraph :one
INSERT INTO relation_graph_favorites(user_id,title,db_name,root_article_id,node_count,graph,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?) RETURNING id,title,db_name,root_article_id,node_count,created_at,updated_at,graph;

-- name: Graph :one
SELECT id,title,db_name,root_article_id,node_count,created_at,updated_at,graph FROM relation_graph_favorites WHERE user_id=? AND id=?;

-- name: DeleteGraph :execresult
DELETE FROM relation_graph_favorites WHERE user_id=? AND id=?;

