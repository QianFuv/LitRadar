-- name: RecordHeartbeat :execresult
INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES(?,?,?,?) ON CONFLICT(service,instance_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at;

-- name: PruneHeartbeats :execresult
DELETE FROM service_heartbeats WHERE heartbeat_at<?;

-- name: DeleteHeartbeat :execresult
DELETE FROM service_heartbeats WHERE service=? AND instance_id=?;
