package scholarly

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"math"
	"strconv"
	"strings"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/transport"
)

// IssueGroup carries a safe issue identity and the group's maximum sorting date.
type IssueGroup struct {
	Anchor Anchor
	Date   string
}

// EmissionPage retains the core-owned keyset cursor without changing the local sealed state.
type EmissionPage struct {
	Works   []any
	After   *EmissionKey
	HasMore bool
}

// SealSelection persists the chosen issue window before canonical article emission.
func (workset *CrossrefWorkset) SealSelection(upper string, lower *string, candidate *Anchor) (CrossrefCheckpoint, error) {
	if workset.state.Phase.Kind != "ready" {
		return CrossrefCheckpoint{}, invalidWorkset("Crossref selection is already sealed")
	}
	next := workset.state.Clone()
	if next.Sequence == math.MaxUint64 {
		return CrossrefCheckpoint{}, invalidWorkset("Crossref sequence overflow")
	}
	next.Sequence++
	next.Candidate = copyAnchor(candidate)
	next.Phase = CrossrefPhase{Kind: "emit", Upper: upper, Lower: clonePointer(lower)}
	if err := next.Validate(); err != nil {
		return CrossrefCheckpoint{}, err
	}
	if err := workset.saveState(next, workset.state); err != nil {
		return CrossrefCheckpoint{}, err
	}
	workset.state = next
	return workset.Checkpoint(), nil
}

// FirstGroup selects the newest safe issue in the complete locally sorted result.
func (workset *CrossrefWorkset) FirstGroup() (*IssueGroup, error) {
	return workset.readGroup("SELECT anchor,date FROM groups WHERE anchor IS NOT NULL ORDER BY rank LIMIT 1")
}

// Group resolves identity independently of the supplied anchor's synchronization date.
func (workset *CrossrefWorkset) Group(anchor Anchor) (*IssueGroup, error) {
	key, err := anchor.Issue.MarshalJSON()
	if err != nil {
		return nil, worksetStorageError(err)
	}
	return workset.readGroup("SELECT anchor,date FROM groups WHERE fingerprint=?1", string(key))
}

func (workset *CrossrefWorkset) readGroup(query string, values ...any) (*IssueGroup, error) {
	var encoded, date storage.Text
	err := workset.row(query, values...).Scan(&encoded, &date)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, worksetStorageError(err)
	}
	var anchor Anchor
	if json.Unmarshal([]byte(encoded), &anchor) != nil {
		return nil, invalidWorkset("invalid Crossref workset metadata")
	}
	return &IssueGroup{anchor, string(date)}, nil
}

// HasUnknownGroups detects records without a safe issue identity before window selection.
func (workset *CrossrefWorkset) HasUnknownGroups() (bool, error) {
	var result storage.Integer
	err := workset.row("SELECT EXISTS(SELECT 1 FROM groups WHERE anchor IS NULL)").Scan(&result)
	return result != 0, worksetStorageError(err)
}

// Emit reads at most 225 works and 16 MiB, selecting whole issue groups by their maximum date.
func (workset *CrossrefWorkset) Emit(upper string, lower *string, after *EmissionKey) (EmissionPage, error) {
	page := EmissionPage{Works: []any{}, After: clonePointer(after)}
	rank, sortDate, key, err := workset.emissionStart(after)
	if err != nil {
		return page, err
	}
	rows, err := workset.connection.QueryContext(context.Background(), `SELECT w.rowid,w.group_rank,w.date,w.key,length(CAST(w.payload AS BLOB)) FROM works w INDEXED BY works_emit JOIN groups g ON g.rank=w.group_rank
WHERE (w.group_rank,w.sort_date,w.key)>(?3,?4,?5) AND g.date<=?1 AND (?2 IS NULL OR g.date>=?2)
ORDER BY w.group_rank,w.sort_date,w.key LIMIT 226`, upper, lower, rank, sortDate, key)
	if err != nil {
		return page, worksetStorageError(err)
	}
	defer rows.Close()
	bytes := 0
	for rows.Next() {
		var rowId, rankValue, dateValue, keyValue any
		var size storage.Integer
		if err := rows.Scan(&rowId, &rankValue, &dateValue, &keyValue, &size); err != nil {
			return page, worksetStorageError(err)
		}
		shouldStop, err := checkEmissionBudget(size, bytes, len(page.Works))
		if err != nil {
			return page, err
		}
		if shouldStop {
			page.HasMore = true
			break
		}
		decoded, position, payloadBytes, err := workset.readEmissionWork(rowId, rankValue, dateValue, keyValue)
		if err != nil {
			return page, err
		}
		bytes += payloadBytes
		page.Works = append(page.Works, decoded)
		page.After = position
	}
	if err := rows.Err(); err != nil {
		return page, worksetStorageError(err)
	}
	return page, nil
}

// emissionStart validates cursor existence before deriving the indexed tuple position.
func (workset *CrossrefWorkset) emissionStart(after *EmissionKey) (int64, int64, string, error) {
	var rank, sortDate int64
	key := ""
	if after != nil {
		var exists storage.Integer
		if err := workset.row("SELECT EXISTS(SELECT 1 FROM works WHERE key=?1 AND group_rank=?2 AND date=?3)", after.Key, after.Rank, after.Date).Scan(&exists); err != nil {
			return 0, 0, "", worksetStorageError(err)
		}
		if exists == 0 {
			return 0, 0, "", invalidWorkset("Crossref emission key is not present in its verified workset")
		}
		rank, key = after.Rank, after.Key
		if parsed, err := strconv.ParseInt(strings.ReplaceAll(after.Date, "-", ""), 10, 64); err == nil {
			sortDate = -parsed
		}
	}
	return rank, sortDate, key, nil
}

// checkEmissionBudget rejects negative sizes and excludes over-budget rows before payload or cursor typing.
func checkEmissionBudget(size storage.Integer, usedBytes, count int) (bool, error) {
	if size < 0 {
		return false, worksetStorageError(errors.New("integer out of range"))
	}
	if count == 225 || int64(size) > 16*1024*1024-int64(usedBytes) {
		if count == 0 {
			return false, invalidWorkset("Crossref work exceeds the local emission size limit")
		}
		return true, nil
	}
	return false, nil
}

// readEmissionWork loads and decodes an admitted payload before validating its cursor field types.
func (workset *CrossrefWorkset) readEmissionWork(rowId, rankValue, dateValue, keyValue any) (any, *EmissionKey, int, error) {
	var payload, date, key storage.Text
	var rank storage.Integer
	if err := workset.row("SELECT payload FROM works WHERE rowid=?1", rowId).Scan(&payload); err != nil {
		return nil, nil, 0, worksetStorageError(err)
	}
	decoded, err := transport.ParseJson([]byte(payload))
	if err != nil {
		return nil, nil, 0, invalidWorkset("invalid Crossref workset metadata")
	}
	if err := rank.Scan(rankValue); err != nil {
		return nil, nil, 0, worksetStorageError(err)
	}
	if err := date.Scan(dateValue); err != nil {
		return nil, nil, 0, worksetStorageError(err)
	}
	if err := key.Scan(keyValue); err != nil {
		return nil, nil, 0, worksetStorageError(err)
	}
	return decoded, &EmissionKey{int64(rank), string(date), string(key)}, len(payload), nil
}
