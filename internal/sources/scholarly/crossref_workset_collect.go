package scholarly

import (
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"math"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

func (workset *CrossrefWorkset) saveState(next, previous CrossrefCheckpoint) error {
	encodedNext, err := next.MarshalJSON()
	if err != nil {
		return worksetStorageError(err)
	}
	encodedPrevious, err := previous.MarshalJSON()
	if err != nil {
		return worksetStorageError(err)
	}
	return workset.exec("UPDATE metadata SET state=?1,previous=?2 WHERE id=1", string(encodedNext), string(encodedPrevious))
}

// Accept atomically commits one response with its counters and replayable previous checkpoint.
func (workset *CrossrefWorkset) Accept(page CrossrefPage) (CrossrefCheckpoint, error) {
	previous := workset.state.Clone()
	if err := workset.exec("BEGIN IMMEDIATE"); err != nil {
		return CrossrefCheckpoint{}, err
	}
	err := workset.acceptResponse(page)
	if err == nil {
		if previous.Sequence == math.MaxUint64 {
			err = invalidWorkset("Crossref sequence overflow")
		} else {
			workset.state.Sequence = previous.Sequence + 1
			err = workset.state.Validate()
		}
	}
	if err == nil {
		err = workset.saveState(workset.state, previous)
	}
	if err == nil {
		err = workset.exec("COMMIT")
	}
	if err != nil {
		workset.state = previous
		isAutocommit := false
		if rawErr := workset.connection.Raw(func(connection any) error { isAutocommit = connection.(*native.SQLiteConn).AutoCommit(); return nil }); rawErr != nil {
			return CrossrefCheckpoint{}, worksetStorageError(rawErr)
		}
		if !isAutocommit {
			if rollbackErr := workset.exec("ROLLBACK"); rollbackErr != nil {
				return CrossrefCheckpoint{}, rollbackErr
			}
		}
		return CrossrefCheckpoint{}, err
	}
	return workset.Checkpoint(), nil
}

func (workset *CrossrefWorkset) acceptResponse(page CrossrefPage) error {
	if page.TotalResults > math.MaxInt64 {
		return invalidWorkset("Crossref count exceeds the supported range")
	}
	phase := workset.state.Phase
	switch phase.Kind {
	case "discover":
		return workset.acceptDiscovery(page)
	case "collect":
		return workset.acceptCollection(page, phase)
	default:
		return invalidWorkset("Crossref response arrived after collection was verified")
	}
}

func (workset *CrossrefWorkset) acceptCollection(page CrossrefPage, phase CrossrefPhase) error {
	if didAdvance, err := workset.acceptCollectionCounts(page, phase); err != nil || didAdvance {
		return err
	}
	if didAdvance, err := workset.prepareCollectionProbe(page, phase); err != nil || didAdvance {
		return err
	}
	totalReceived, didAdvance, err := workset.collectionReceivedCount(page, phase)
	if err != nil || didAdvance {
		return err
	}
	isTerminal := isTerminalCrossrefPage(page, phase)
	prepared, didAdvance, err := workset.prepareCollectionWorks(page.Items, phase)
	if err != nil || didAdvance {
		return err
	}
	if err := workset.insertCollectionWorks(prepared, phase.Partition); err != nil {
		return err
	}
	return workset.finishCollectionPage(page, phase, totalReceived, isTerminal)
}

// isTerminalCrossrefPage requires a short cursor page even when a full page reaches the total.
func isTerminalCrossrefPage(page CrossrefPage, phase CrossrefPhase) bool {
	return phase.Cursor == nil || len(page.Items) < 225
}

func splitSecond(first, last int64) (int64, error) {
	if first >= last || first < 0 && last > math.MaxInt64+first {
		return 0, invalidWorkset("Crossref partition cannot be split safely")
	}
	return first + (last-first)/2, nil
}

func (workset *CrossrefWorkset) retryLeaf(partition int64, retry uint8, reason string) error {
	if retry >= 1 {
		return invalidWorkset("Crossref partition drift persisted after one retry: " + reason)
	}
	if err := workset.exec("DELETE FROM works WHERE leaf=?1", partition); err != nil {
		return err
	}
	if err := workset.exec("UPDATE partitions SET retry=retry+1,received=0,cursor=CASE WHEN status='cursor' THEN '*' ELSE NULL END WHERE id=?1", partition); err != nil {
		return err
	}
	slog.Warn("source.crossref.partition_retried", "reason", reason, "generation", workset.state.Generation)
	return workset.nextPartition()
}

func (workset *CrossrefWorkset) restartGeneration(reason string) error {
	if workset.state.Generation >= 1 {
		return invalidWorkset("Crossref journal count drift persisted after one retry: " + reason)
	}
	if err := workset.exec("DELETE FROM works; DELETE FROM partitions; DELETE FROM groups;"); err != nil {
		return err
	}
	workset.state.Generation++
	workset.state.RootTotal = nil
	if err := workset.initializeRoot(); err != nil {
		return err
	}
	workset.state.restartCollection()
	slog.Warn("source.crossref.generation_retried", "reason", reason, "generation", workset.state.Generation)
	return nil
}

func (workset *CrossrefWorkset) nextPartition() error {
	var partition, first, last, received, retry storage.Integer
	var cursor storage.OptionalText
	var expected storage.OptionalInteger
	err := workset.row("SELECT id,first_second,last_second,cursor,received,expected,retry FROM partitions WHERE status IN ('probe','cursor') ORDER BY first_second,id LIMIT 1").Scan(&partition, &first, &last, &cursor, &received, &expected, &retry)
	if err == nil {
		return workset.resumeCollectionPartition(partition, first, last, received, retry, cursor, expected)
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return worksetStorageError(err)
	}
	var invalidParents, unique storage.Integer
	if err := workset.row("SELECT count(*) FROM partitions p WHERE p.status='split' AND p.expected != (SELECT sum(c.expected) FROM partitions c WHERE c.parent=p.id)").Scan(&invalidParents); err != nil {
		return worksetStorageError(err)
	}
	if err := workset.row("SELECT count(*) FROM works").Scan(&unique); err != nil {
		return worksetStorageError(err)
	}
	if invalidParents != 0 || workset.state.RootTotal == nil || uint64(unique) != *workset.state.RootTotal {
		return workset.restartGeneration("partition tree does not reconcile with the global unique count")
	}
	return workset.buildCollectionGroups()
}

// acceptDiscovery freezes the first creation bound and preserves singleton recursive collection.
func (workset *CrossrefWorkset) acceptDiscovery(page CrossrefPage) error {
	if page.TotalResults == 0 && len(page.Items) == 0 {
		workset.state.RootTotal = new(uint64(0))
		workset.state.Phase = CrossrefPhase{Kind: "ready"}
		return nil
	}
	if page.TotalResults == 0 || len(page.Items) != 1 {
		return invalidWorkset("Crossref creation discovery is incomplete")
	}
	from, err := createdSecond(page.Items[0])
	if err != nil {
		return err
	}
	if from > workset.state.FrozenAt {
		return invalidWorkset("Crossref creation discovery exceeds the frozen bound")
	}
	workset.state.CreatedFrom = &from
	if err := workset.initializeRoot(); err != nil {
		return err
	}
	workset.state.restartCollection()
	if workset.state.UpdatedFrom == nil && page.TotalResults == 1 {
		return workset.acceptResponse(page)
	}
	return nil
}

// acceptCollectionCounts preserves root drift publication before partition-count retry classification.
func (workset *CrossrefWorkset) acceptCollectionCounts(page CrossrefPage, phase CrossrefPhase) (bool, error) {
	if phase.Partition == 1 && phase.Cursor == nil {
		if workset.state.RootTotal != nil && *workset.state.RootTotal != page.TotalResults {
			return true, workset.restartGeneration("root count changed")
		}
		workset.state.RootTotal = clonePointer(&page.TotalResults)
	}
	if phase.Expected != nil && *phase.Expected != page.TotalResults {
		return true, workset.retryLeaf(phase.Partition, phase.Retry, "partition count changed")
	}
	return false, nil
}

// splitCollectionPartition validates depth and safe midpoint before inserting both child bounds.
func (workset *CrossrefWorkset) splitCollectionPartition(phase CrossrefPhase) error {
	var depth storage.Integer
	if err := workset.row("SELECT depth FROM partitions WHERE id=?1", phase.Partition).Scan(&depth); err != nil {
		return worksetStorageError(err)
	}
	if depth < 0 || depth > math.MaxUint32 {
		return worksetStorageError(errors.New("integer out of range"))
	}
	if depth >= 64 {
		return invalidWorkset("Crossref partition depth exceeded")
	}
	middle, err := splitSecond(phase.From, phase.Until)
	if err != nil {
		return err
	}
	if err := workset.exec("UPDATE partitions SET status='split' WHERE id=?1", phase.Partition); err != nil {
		return err
	}
	for _, bounds := range [][2]int64{{phase.From, middle}, {middle + 1, phase.Until}} {
		if err := workset.exec("INSERT INTO partitions(parent,first_second,last_second,depth,status) VALUES(?1,?2,?3,?4,'probe')", phase.Partition, bounds[0], bounds[1], int64(depth)+1); err != nil {
			return err
		}
	}
	return nil
}

// prepareCollectionProbe updates expected counts before probe truncation and split or cursor advancement.
func (workset *CrossrefWorkset) prepareCollectionProbe(page CrossrefPage, phase CrossrefPhase) (bool, error) {
	if phase.Cursor == nil {
		if err := workset.exec("UPDATE partitions SET expected=?1 WHERE id=?2", page.TotalResults, phase.Partition); err != nil {
			return false, err
		}
		if uint64(len(page.Items)) != min(page.TotalResults, 225) {
			return true, workset.retryLeaf(phase.Partition, phase.Retry, "single response was truncated")
		}
		if page.TotalResults > 225 {
			return true, workset.advanceLargeCollectionProbe(phase)
		}
	} else if len(page.Items) > 225 {
		return true, workset.retryLeaf(phase.Partition, phase.Retry, "cursor page exceeds the requested size")
	}
	return false, nil
}

// preparedWork owns the exact consumed payload and serialized identity before any insertion.
type preparedWork struct {
	key        string
	payload    map[string]any
	serialized string
}

// prepareCollectionWork checks creation bounds, exact payload identity and existing leaf ownership.
func (workset *CrossrefWorkset) prepareCollectionWork(work any, phase CrossrefPhase, keys map[string]bool) (preparedWork, bool, error) {
	created, err := createdSecond(work)
	if err != nil {
		return preparedWork{}, false, err
	}
	if created < phase.From || created > phase.Until {
		return preparedWork{}, true, workset.retryLeaf(phase.Partition, phase.Retry, "work moved outside its creation partition")
	}
	payload := consumedPayload(work)
	serialized, err := domain.Json(payload)
	if err != nil {
		return preparedWork{}, false, worksetStorageError(err)
	}
	key := workKey(payload, string(serialized))
	if keys[key] {
		return preparedWork{}, true, workset.retryLeaf(phase.Partition, phase.Retry, "duplicate key inside one response")
	}
	keys[key] = true
	var leaf storage.Integer
	err = workset.row("SELECT leaf FROM works WHERE key=?1", key).Scan(&leaf)
	if err == nil {
		if int64(leaf) != phase.Partition {
			return preparedWork{}, true, workset.restartGeneration("duplicate key across creation partitions")
		}
		return preparedWork{}, true, workset.retryLeaf(phase.Partition, phase.Retry, "cursor made no unique progress or repeated a key")
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return preparedWork{}, false, worksetStorageError(err)
	}
	return preparedWork{key, payload, string(serialized)}, false, nil
}

// prepareCollectionWorks validates every response item before any ordering calculation or insertion.
func (workset *CrossrefWorkset) prepareCollectionWorks(items []any, phase CrossrefPhase) ([]preparedWork, bool, error) {
	prepared := make([]preparedWork, 0, len(items))
	keys := map[string]bool{}
	for _, work := range items {
		item, didAdvance, err := workset.prepareCollectionWork(work, phase, keys)
		if err != nil || didAdvance {
			return nil, didAdvance, err
		}
		prepared = append(prepared, item)
	}
	return prepared, false, nil
}

// insertCollectionWorks calculates ordering and writes each prepared payload in response order.
func (workset *CrossrefWorkset) insertCollectionWorks(prepared []preparedWork, partition int64) error {
	for _, work := range prepared {
		order, err := crossrefOrder(work.payload, work.key)
		if err != nil {
			return err
		}
		if err := workset.exec("INSERT INTO works(key,payload,leaf,date,fingerprint,anchor,year,volume,issue) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", work.key, work.serialized, partition, order.Date, order.Fingerprint, order.Anchor, order.Year, order.Volume, order.Issue); err != nil {
			return err
		}
	}
	return nil
}

// finishCollectionPage checks terminal cursor metadata only after all prepared inserts.
func (workset *CrossrefWorkset) finishCollectionPage(page CrossrefPage, phase CrossrefPhase, totalReceived uint64, isTerminal bool) error {
	isDone := totalReceived == page.TotalResults && isTerminal
	var cursor *string
	status := "done"
	if !isDone {
		if page.NextCursor == nil || *page.NextCursor == "" {
			return workset.retryLeaf(phase.Partition, phase.Retry, "cursor ended without a verified terminal response")
		}
		cursor = page.NextCursor
		status = "cursor"
	}
	if err := workset.exec("UPDATE partitions SET expected=?1,received=?2,cursor=?3,status=?4 WHERE id=?5", page.TotalResults, totalReceived, cursor, status, phase.Partition); err != nil {
		return err
	}
	return workset.nextPartition()
}

// resumeCollectionPartition validates SQL counters before publishing the next request phase.
func (workset *CrossrefWorkset) resumeCollectionPartition(partition, first, last, received, retry storage.Integer, cursor storage.OptionalText, expected storage.OptionalInteger) error {
	if received < 0 || retry < 0 || retry > math.MaxUint8 || expected.Value != nil && *expected.Value < 0 {
		return worksetStorageError(errors.New("integer out of range"))
	}
	phase := CrossrefPhase{Kind: "collect", Partition: int64(partition), From: int64(first), Until: int64(last), Cursor: cursor.Value, Received: uint64(received), Retry: uint8(retry)}
	if expected.Value != nil {
		phase.Expected = new(uint64(*expected.Value))
	}
	workset.state.Phase = phase
	return nil
}

// buildCollectionGroups constructs deterministic group ranks before publishing the ready phase.
func (workset *CrossrefWorkset) buildCollectionGroups() error {
	if err := workset.exec("INSERT INTO groups(fingerprint,anchor,date,year,volume,issue) SELECT fingerprint,min(anchor),max(date),max(year),max(volume),max(issue) FROM works INDEXED BY works_order GROUP BY fingerprint;"); err != nil {
		return err
	}
	rows, err := workset.connection.QueryContext(context.Background(), "SELECT fingerprint FROM groups INDEXED BY groups_order ORDER BY date DESC,year DESC,volume DESC,issue DESC,fingerprint")
	if err != nil {
		return worksetStorageError(err)
	}
	defer rows.Close()
	var rank int64
	for rows.Next() {
		rank++
		var fingerprint storage.Text
		if err := rows.Scan(&fingerprint); err != nil {
			return worksetStorageError(err)
		}
		if err := workset.exec("UPDATE groups SET rank=?1 WHERE fingerprint=?2", rank, string(fingerprint)); err != nil {
			return err
		}
	}
	if err := rows.Err(); err != nil {
		return worksetStorageError(err)
	}
	if err := rows.Close(); err != nil {
		return worksetStorageError(err)
	}
	if err := workset.exec("UPDATE works SET group_rank=(SELECT rank FROM groups WHERE groups.fingerprint=works.fingerprint)"); err != nil {
		return err
	}
	workset.state.Phase = CrossrefPhase{Kind: "ready"}
	return nil
}

// collectionReceivedCount rejects overflow and incomplete cursor counts before payload preparation.
func (workset *CrossrefWorkset) collectionReceivedCount(page CrossrefPage, phase CrossrefPhase) (uint64, bool, error) {
	if uint64(len(page.Items)) > math.MaxUint64-phase.Received {
		return 0, false, invalidWorkset("Crossref received count overflow")
	}
	totalReceived := phase.Received + uint64(len(page.Items))
	if totalReceived > page.TotalResults || len(page.Items) == 0 && totalReceived != page.TotalResults || phase.Cursor != nil && len(page.Items) < 225 && totalReceived != page.TotalResults {
		return 0, true, workset.retryLeaf(phase.Partition, phase.Retry, "cursor count is incomplete or exceeds the expected count")
	}
	return totalReceived, false, nil
}

// advanceLargeCollectionProbe chooses split children or a singleton cursor before selecting the next partition.
func (workset *CrossrefWorkset) advanceLargeCollectionProbe(phase CrossrefPhase) error {
	if phase.From < phase.Until {
		if err := workset.splitCollectionPartition(phase); err != nil {
			return err
		}
	} else if err := workset.exec("UPDATE partitions SET status='cursor',cursor='*' WHERE id=?1", phase.Partition); err != nil {
		return err
	}
	return workset.nextPartition()
}
