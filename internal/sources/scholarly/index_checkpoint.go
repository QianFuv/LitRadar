package scholarly

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

type indexWindow struct {
	SyncMode            domain.IndexSyncMode `json:"sync_mode"`
	Phase               string               `json:"phase"`
	BaseAnchor          *Anchor              `json:"base_anchor,omitempty"`
	CandidateAnchor     *Anchor              `json:"candidate_anchor,omitempty"`
	HasReachedCandidate bool                 `json:"has_reached_candidate"`
	HasSeenBase         bool                 `json:"has_seen_base"`
}

func (window *indexWindow) UnmarshalJSON(body []byte) error {
	type wire struct {
		SyncMode            indexUnit `json:"sync_mode"`
		Phase               indexUnit `json:"phase"`
		BaseAnchor          *Anchor   `json:"base_anchor,omitempty"`
		CandidateAnchor     *Anchor   `json:"candidate_anchor,omitempty"`
		HasReachedCandidate bool      `json:"has_reached_candidate"`
		HasSeenBase         bool      `json:"has_seen_base"`
	}
	var decoded wire
	if err := decodeWorksetStruct(body, &decoded); err != nil {
		return err
	}
	mode := domain.IndexSyncMode(decoded.SyncMode)
	if mode != domain.Bootstrap && mode != domain.Incremental && mode != domain.FullRescan || decoded.Phase != "bounded" && decoded.Phase != "unbounded" {
		return errWorksetJson
	}
	*window = indexWindow{mode, string(decoded.Phase), decoded.BaseAnchor, decoded.CandidateAnchor, decoded.HasReachedCandidate, decoded.HasSeenBase}
	return nil
}

type indexUnit string

func (unit *indexUnit) UnmarshalJSON(body []byte) error {
	body = bytes.TrimSpace(body)
	var name string
	if len(body) > 0 && body[0] == '"' {
		if json.Unmarshal(body, &name) != nil {
			return errWorksetJson
		}
	} else {
		decoder := json.NewDecoder(bytes.NewReader(body))
		opening, err := decoder.Token()
		if err != nil || opening != json.Delim('{') || !decoder.More() {
			return errWorksetJson
		}
		key, err := decoder.Token()
		if err != nil {
			return errWorksetJson
		}
		var value json.RawMessage
		if decoder.Decode(&value) != nil || !bytes.Equal(bytes.TrimSpace(value), []byte("null")) || decoder.More() {
			return errWorksetJson
		}
		name = key.(string)
	}
	*unit = indexUnit(name)
	return nil
}

func (window indexWindow) clone() indexWindow {
	window.BaseAnchor = copyAnchor(window.BaseAnchor)
	window.CandidateAnchor = copyAnchor(window.CandidateAnchor)
	return window
}

type indexSource struct {
	Kind              string
	State             *CrossrefCheckpoint
	Issn              string
	Cursor            *string
	PageIndex         uint64
	CursorRefreshedAt *uint64
	SourceId          string
}

type worksetSourceWire struct {
	Kind  string             `json:"kind"`
	State CrossrefCheckpoint `json:"state"`
}
type legacySourceWire struct {
	Kind              string  `json:"kind"`
	Issn              string  `json:"issn"`
	Cursor            *string `json:"cursor,omitempty"`
	PageIndex         uint64  `json:"page_index"`
	CursorRefreshedAt *uint64 `json:"cursor_refreshed_at_epoch_seconds,omitempty"`
}
type openAlexSourceWire struct {
	Kind     string  `json:"kind"`
	SourceId string  `json:"source_id"`
	Cursor   *string `json:"cursor,omitempty"`
}

func (source indexSource) MarshalJSON() ([]byte, error) {
	switch source.Kind {
	case "crossref_workset":
		if source.State == nil {
			return nil, errWorksetJson
		}
		return encodeWorksetStruct(worksetSourceWire{source.Kind, *source.State})
	case "crossref":
		return encodeWorksetStruct(legacySourceWire{source.Kind, source.Issn, source.Cursor, source.PageIndex, source.CursorRefreshedAt})
	case "open_alex":
		return encodeWorksetStruct(openAlexSourceWire{source.Kind, source.SourceId, source.Cursor})
	default:
		return nil, errWorksetJson
	}
}

func (source *indexSource) UnmarshalJSON(body []byte) error {
	kind, err := worksetKind(body)
	if err != nil {
		return err
	}
	decoded := indexSource{Kind: kind}
	switch kind {
	case "crossref_workset":
		var wire worksetSourceWire
		if err := decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.State = &wire.State
	case "crossref":
		var wire legacySourceWire
		if err := decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.Issn, decoded.Cursor, decoded.PageIndex, decoded.CursorRefreshedAt = wire.Issn, wire.Cursor, wire.PageIndex, wire.CursorRefreshedAt
	case "open_alex":
		var wire openAlexSourceWire
		if err := decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.SourceId, decoded.Cursor = wire.SourceId, wire.Cursor
	default:
		return errWorksetJson
	}
	*source = decoded
	return nil
}

type indexCheckpoint struct {
	Version uint32      `json:"version"`
	Window  indexWindow `json:"window"`
	Source  indexSource `json:"source"`
}

func (checkpoint *indexCheckpoint) UnmarshalJSON(body []byte) error {
	type wire indexCheckpoint
	var decoded wire
	if err := decodeWorksetStruct(body, &decoded); err != nil {
		return err
	}
	*checkpoint = indexCheckpoint(decoded)
	return nil
}

func encodeIndexCheckpoint(checkpoint indexCheckpoint) (string, error) {
	encoded, err := encodeWorksetStruct(checkpoint)
	if err != nil {
		return "", &provider.Error{Kind: provider.Internal, Message: "scholarly checkpoint could not be encoded"}
	}
	if len(encoded) > 65536 {
		return "", invalidWorkset("scholarly checkpoint exceeds the provider contract limit")
	}
	return string(encoded), nil
}

func decodeIndexAnchor(raw string) (*Anchor, error) {
	var anchor Anchor
	if json.Unmarshal([]byte(raw), &anchor) != nil || !anchor.IsValid() {
		return nil, invalidWorkset("scholarly committed anchor is invalid")
	}
	return &anchor, nil
}

func decodeIndexCheckpoint(raw string, currentDate *string) (indexCheckpoint, error) {
	var checkpoint indexCheckpoint
	if len(raw) > 65536 {
		return checkpoint, invalidWorkset("scholarly checkpoint exceeds the provider contract limit")
	}
	if json.Unmarshal([]byte(raw), &checkpoint) != nil {
		return checkpoint, invalidWorkset("scholarly checkpoint is invalid")
	}
	normalizeIndexReplay(&checkpoint, currentDate)
	source, window := checkpoint.Source, checkpoint.Window
	isSourceValid := false
	switch source.Kind {
	case "crossref_workset":
		isSourceValid = checkpoint.Version == 2 && source.State != nil && source.State.Validate() == nil
	case "crossref":
		isSourceValid = checkpoint.Version == 1 && strings.TrimSpace(source.Issn) != "" && (source.Cursor == nil && source.PageIndex == 0 && source.CursorRefreshedAt == nil || source.Cursor != nil && *source.Cursor != "" && source.CursorRefreshedAt != nil)
	case "open_alex":
		isSourceValid = strings.TrimSpace(source.SourceId) != "" && (source.Cursor == nil || *source.Cursor != "")
	}
	isWindowValid := (window.BaseAnchor == nil || window.BaseAnchor.IsValid()) && (window.CandidateAnchor == nil || window.CandidateAnchor.IsValid()) && (window.Phase != "bounded" || window.BaseAnchor != nil && window.BaseAnchor.FromSyncDate != nil) && (window.CandidateAnchor != nil || !window.HasReachedCandidate && !window.HasSeenBase) && (!window.HasReachedCandidate || window.CandidateAnchor != nil) && (!window.HasSeenBase || window.Phase == "bounded" && window.HasReachedCandidate)
	if checkpoint.Version != 1 && checkpoint.Version != 2 || !isSourceValid || !isWindowValid {
		return checkpoint, invalidWorkset("scholarly checkpoint is invalid")
	}
	return checkpoint, nil
}

func normalizeIndexReplay(checkpoint *indexCheckpoint, currentDate *string) {
	source := checkpoint.Source
	isHead := source.Kind == "crossref" && source.Cursor == nil && source.PageIndex == 0 && source.CursorRefreshedAt == nil || source.Kind == "open_alex" && source.Cursor == nil
	window := &checkpoint.Window
	if (checkpoint.Version == 1 || checkpoint.Version == 2) && window.Phase == "unbounded" && window.BaseAnchor != nil && window.CandidateAnchor == nil && window.HasReachedCandidate && !window.HasSeenBase && isHead {
		window.HasReachedCandidate = false
		if currentDate != nil && window.BaseAnchor.FromSyncDate != nil && *window.BaseAnchor.FromSyncDate > *currentDate {
			window.Phase = "bounded"
		}
	}
}

func indexWindowFromContext(context domain.IndexFetchContext, currentDate *string) (indexWindow, *indexSource, error) {
	var anchor *Anchor
	var err error
	if context.Mode == domain.Incremental && context.CommittedAnchor != nil {
		anchor, err = decodeIndexAnchor(*context.CommittedAnchor)
		if err != nil {
			return indexWindow{}, nil, err
		}
	}
	if context.TraversalCheckpoint != nil {
		checkpoint, err := decodeIndexCheckpoint(*context.TraversalCheckpoint, currentDate)
		if err != nil {
			return indexWindow{}, nil, err
		}
		if checkpoint.Window.SyncMode != context.Mode || !reflect.DeepEqual(checkpoint.Window.BaseAnchor, anchor) {
			return indexWindow{}, nil, invalidWorkset("scholarly checkpoint does not match the frozen synchronization window")
		}
		return checkpoint.Window, &checkpoint.Source, nil
	}
	phase := "unbounded"
	if anchor != nil && anchor.FromSyncDate != nil {
		phase = "bounded"
	}
	return indexWindow{SyncMode: context.Mode, Phase: phase, BaseAnchor: anchor}, nil, nil
}

func indexWindowFilter(window indexWindow, currentDate *string) *string {
	if window.Phase != "bounded" || window.BaseAnchor == nil || window.BaseAnchor.FromSyncDate == nil {
		return nil
	}
	from := window.BaseAnchor.FromSyncDate
	if currentDate != nil && *from > *currentDate {
		return nil
	}
	return clonePointer(from)
}

func prepareIndexReplay(window *indexWindow) {
	window.HasReachedCandidate = false
	window.HasSeenBase = false
}
func unboundIndexWindow(window *indexWindow) { window.Phase = "unbounded"; prepareIndexReplay(window) }

func indexWorksetScope(catalog domain.JournalCatalogEntry, window indexWindow) (string, error) {
	type catalogWire domain.JournalCatalogEntry
	encodedCatalog, err := encodeWorksetStruct(catalogWire(catalog))
	if err != nil {
		return "", &provider.Error{Kind: provider.Internal, Message: "Crossref frozen context could not be encoded"}
	}
	mode, err := domain.Json(string(window.SyncMode))
	if err != nil {
		return "", err
	}
	anchor := []byte("null")
	if window.BaseAnchor != nil {
		anchor, err = window.BaseAnchor.MarshalJSON()
		if err != nil {
			return "", err
		}
	}
	return "[" + string(encodedCatalog) + "," + string(mode) + "," + string(anchor) + "]", nil
}
