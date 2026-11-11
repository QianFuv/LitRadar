-- name: BuildGroups :exec
INSERT INTO groups(fingerprint,anchor,date,year,volume,issue) SELECT fingerprint,min(anchor),max(date),max(year),max(volume),max(issue) FROM works INDEXED BY works_order GROUP BY fingerprint;

-- name: ClearGroups :exec
DELETE FROM groups;

-- name: ClearPartitions :exec
DELETE FROM partitions;

-- name: ClearWorks :exec
DELETE FROM works;

-- name: DeletePartitionWorks :exec
DELETE FROM works WHERE leaf=?1;

-- name: EmissionRows :many
SELECT w.group_rank,w.date,w.key,length(CAST(w.payload AS BLOB)) FROM works w INDEXED BY works_emit JOIN groups g ON g.rank=w.group_rank
WHERE (w.group_rank,w.sort_date,w.key)>(CAST(?3 AS INTEGER),CAST(?4 AS INTEGER),CAST(?5 AS TEXT)) AND g.date<=?1 AND (?2 IS NULL OR g.date>=?2)
ORDER BY w.group_rank,w.sort_date,w.key LIMIT 226;

-- name: FinishPartitionPage :exec
UPDATE partitions SET expected=?1,received=?2,cursor=?3,status=?4 WHERE id=?5;

-- name: FirstGroup :one
SELECT anchor,date FROM groups WHERE anchor IS NOT NULL ORDER BY rank LIMIT 1;

-- name: GroupByFingerprint :one
SELECT anchor,date FROM groups WHERE fingerprint=?1;

-- name: HasEmissionKey :one
SELECT EXISTS(SELECT 1 FROM works WHERE key=?1 AND group_rank=?2 AND date=?3);

-- name: HasUnknownGroups :one
SELECT EXISTS(SELECT 1 FROM groups WHERE anchor IS NULL);

-- name: InsertMetadata :exec
INSERT INTO metadata(id,owner,state,previous) VALUES(1, ?1, ?2, NULL);

-- name: InsertPartition :exec
INSERT INTO partitions(parent,first_second,last_second,depth,status) VALUES(?1,?2,?3,?4,'probe');

-- name: InsertRoot :exec
INSERT INTO partitions(id,parent,first_second,last_second,depth,status) VALUES(1,NULL,?1,?2,0,'probe');

-- name: InsertWork :exec
INSERT INTO works(key,payload,leaf,date,fingerprint,anchor,year,volume,issue) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9);

-- name: InvalidParentCount :one
SELECT count(*) FROM partitions p WHERE p.status='split' AND p.expected != (SELECT sum(c.expected) FROM partitions c WHERE c.parent=p.id);

-- name: Metadata :one
SELECT owner,state,previous FROM metadata WHERE id=1;

-- name: NextPartition :one
SELECT id,first_second,last_second,cursor,received,expected,retry FROM partitions WHERE status IN ('probe','cursor') ORDER BY first_second,id LIMIT 1;

-- name: OrderedGroups :many
SELECT fingerprint FROM groups INDEXED BY groups_order ORDER BY date DESC,year DESC,volume DESC,issue DESC,fingerprint;

-- name: PartitionDepth :one
SELECT depth FROM partitions WHERE id=?1;

-- name: RetryPartition :exec
UPDATE partitions SET retry=retry+1,received=0,cursor=CASE WHEN status='cursor' THEN '*' ELSE NULL END WHERE id=?1;

-- name: SetGroupRank :exec
UPDATE groups SET rank=?1 WHERE fingerprint=?2;

-- name: SetPartitionExpected :exec
UPDATE partitions SET expected=?1 WHERE id=?2;

-- name: SetWorkGroupRanks :exec
UPDATE works SET group_rank=(SELECT rank FROM groups WHERE groups.fingerprint=works.fingerprint);

-- name: SplitPartition :exec
UPDATE partitions SET status='split' WHERE id=?1;

-- name: StartPartitionCursor :exec
UPDATE partitions SET status='cursor',cursor='*' WHERE id=?1;

-- name: UpdateMetadata :exec
UPDATE metadata SET state=?1,previous=?2 WHERE id=1;

-- name: WorkCount :one
SELECT count(*) FROM works;

-- name: WorkLeaf :one
SELECT leaf FROM works WHERE key=?1;

-- name: WorkPayload :one
SELECT payload FROM works WHERE key=?1;
