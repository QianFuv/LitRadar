package scholarly

import (
	"crypto/rand"
	"encoding/hex"
	"math"
	"strings"

	"github.com/QianFuv/LitRadar/internal/provider"
)

// EmissionKey is the keyset position within the verified issue ordering.
type EmissionKey struct {
	Rank int64  `json:"rank"`
	Date string `json:"date"`
	Key  string `json:"key"`
}

// MarshalJSON preserves declared key order for persisted cursor identity.
func (key EmissionKey) MarshalJSON() ([]byte, error) {
	type wire EmissionKey
	return encodeWorksetStruct(wire(key))
}

// UnmarshalJSON retains the strict original metadata grammar.
func (key *EmissionKey) UnmarshalJSON(body []byte) error {
	type wire EmissionKey
	var decoded wire
	if err := decodeWorksetStruct(body, &decoded); err != nil {
		return err
	}
	*key = EmissionKey(decoded)
	return nil
}

// CrossrefPhase is the next collection request or immutable emission position.
type CrossrefPhase struct {
	Kind                   string
	Partition, From, Until int64
	Cursor                 *string
	Received               uint64
	Expected               *uint64
	Retry                  uint8
	Upper                  string
	Lower                  *string
	After                  *EmissionKey
}
type simplePhaseWire struct {
	Kind string `json:"kind"`
}
type collectPhaseWire struct {
	Kind      string  `json:"kind"`
	Partition int64   `json:"partition"`
	From      int64   `json:"from"`
	Until     int64   `json:"until"`
	Cursor    *string `json:"cursor"`
	Received  uint64  `json:"received"`
	Expected  *uint64 `json:"expected"`
	Retry     uint8   `json:"retry"`
}
type emitPhaseWire struct {
	Kind  string       `json:"kind"`
	Upper string       `json:"upper"`
	Lower *string      `json:"lower"`
	After *EmissionKey `json:"after"`
}

// MarshalJSON includes required nulls while preserving each tagged variant's field order.
func (phase CrossrefPhase) MarshalJSON() ([]byte, error) {
	switch phase.Kind {
	case "discover", "ready":
		return encodeWorksetStruct(simplePhaseWire{phase.Kind})
	case "collect":
		return encodeWorksetStruct(collectPhaseWire{phase.Kind, phase.Partition, phase.From, phase.Until, phase.Cursor, phase.Received, phase.Expected, phase.Retry})
	case "emit":
		return encodeWorksetStruct(emitPhaseWire{phase.Kind, phase.Upper, phase.Lower, phase.After})
	default:
		return nil, errWorksetJson
	}
}

// UnmarshalJSON rejects fields from other variants before accepting a durable phase.
func (phase *CrossrefPhase) UnmarshalJSON(body []byte) error {
	kind, err := worksetKind(body)
	if err != nil {
		return err
	}
	decoded := CrossrefPhase{Kind: kind}
	switch kind {
	case "discover", "ready":
		if err = decodeUnitPhase(body); err != nil {
			return err
		}
	case "collect":
		var wire collectPhaseWire
		if err = decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.Partition = wire.Partition
		decoded.From = wire.From
		decoded.Until = wire.Until
		decoded.Cursor = wire.Cursor
		decoded.Received = wire.Received
		decoded.Expected = wire.Expected
		decoded.Retry = wire.Retry
	case "emit":
		var wire emitPhaseWire
		if err = decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.Upper = wire.Upper
		decoded.Lower = wire.Lower
		decoded.After = wire.After
	default:
		return errWorksetJson
	}
	*phase = decoded
	return nil
}

// CrossrefCheckpoint contains a frozen, path-free traversal reference owned by the core ACK.
type CrossrefCheckpoint struct {
	Token       string        `json:"token"`
	Issn        string        `json:"issn"`
	FrozenAt    int64         `json:"frozen_at"`
	CreatedFrom *int64        `json:"created_from"`
	UpdatedFrom *string       `json:"updated_from"`
	RootTotal   *uint64       `json:"root_total"`
	Candidate   *Anchor       `json:"candidate"`
	Generation  uint8         `json:"generation"`
	Sequence    uint64        `json:"sequence"`
	Phase       CrossrefPhase `json:"phase"`
}

// MarshalJSON retains exact state bytes for one-operation-ahead recovery.
func (state CrossrefCheckpoint) MarshalJSON() ([]byte, error) {
	type wire CrossrefCheckpoint
	return encodeWorksetStruct(wire(state))
}

// UnmarshalJSON accepts the original strict map and positional struct representations.
func (state *CrossrefCheckpoint) UnmarshalJSON(body []byte) error {
	type wire CrossrefCheckpoint
	var decoded wire
	if err := decodeWorksetStruct(body, &decoded); err != nil {
		return err
	}
	*state = CrossrefCheckpoint(decoded)
	return nil
}
func invalidWorkset(message string) error {
	return &provider.Error{Kind: provider.InvalidResponse, Message: message}
}
func validWorksetToken(value string) bool {
	if len(value) != 32 {
		return false
	}
	for _, character := range []byte(value) {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f') {
			return false
		}
	}
	return true
}
func newWorksetToken() (string, error) {
	bytes := make([]byte, 16)
	if _, err := rand.Read(bytes); err != nil {
		return "", &provider.Error{Kind: provider.Internal, Message: "Crossref workset: " + err.Error()}
	}
	return hex.EncodeToString(bytes), nil
}

// NewCrossrefCheckpoint freezes a new traversal with a cryptographically random basename.
func NewCrossrefCheckpoint(issn string, frozenAt int64, updatedFrom *string) (CrossrefCheckpoint, error) {
	token, err := newWorksetToken()
	if err != nil {
		return CrossrefCheckpoint{}, err
	}
	state := CrossrefCheckpoint{Token: token, Issn: issn, FrozenAt: frozenAt, UpdatedFrom: clonePointer(updatedFrom), Phase: CrossrefPhase{Kind: "discover"}}
	return state, state.Validate()
}
func validTimestamp(value int64) bool { return value >= -8334601228800 && value <= 8210266876799 }

// Validate rejects malformed counters and phase bounds before any filesystem inspection.
func (state CrossrefCheckpoint) Validate() error {
	if !validCrossrefFrozenIdentity(state) || !validCrossrefCreationBounds(state) || !validCrossrefOptionalMetadata(state) {
		return invalidWorkset("Crossref checkpoint has invalid frozen bounds or counters")
	}
	isValid := validCrossrefPhase(state)
	if !isValid {
		return invalidWorkset("Crossref checkpoint phase is inconsistent")
	}
	return nil
}

// Query constructs exactly the network request represented by the confirmed state.
func (state CrossrefCheckpoint) Query() (CrossrefQuery, error) {
	switch state.Phase.Kind {
	case "discover":
		return CrossrefQuery{IsEarliest: true, CreatedUntil: state.FrozenAt}, nil
	case "collect":
		query := CrossrefQuery{CreatedFrom: state.Phase.From, CreatedUntil: state.Phase.Until, UpdatedFrom: clonePointer(state.UpdatedFrom), Cursor: clonePointer(state.Phase.Cursor)}
		if state.UpdatedFrom != nil {
			query.UpdatedUntil = clonePointer(&state.FrozenAt)
		}
		return query, nil
	default:
		return CrossrefQuery{}, invalidWorkset("Crossref workset has no pending network query")
	}
}

// Clone returns independently owned optional metadata for caller retention.
func (state CrossrefCheckpoint) Clone() CrossrefCheckpoint {
	state.CreatedFrom = clonePointer(state.CreatedFrom)
	state.UpdatedFrom = clonePointer(state.UpdatedFrom)
	state.RootTotal = clonePointer(state.RootTotal)
	state.Candidate = copyAnchor(state.Candidate)
	state.Phase.Cursor = clonePointer(state.Phase.Cursor)
	state.Phase.Expected = clonePointer(state.Phase.Expected)
	state.Phase.Lower = clonePointer(state.Phase.Lower)
	state.Phase.After = clonePointer(state.Phase.After)
	return state
}
func (state *CrossrefCheckpoint) restartCollection() {
	state.Phase = CrossrefPhase{Kind: "discover"}
	if state.CreatedFrom != nil {
		state.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: *state.CreatedFrom, Until: state.FrozenAt}
	}
}

// validCrossrefFrozenIdentity checks the token, journal, generation and frozen clock range.
func validCrossrefFrozenIdentity(state CrossrefCheckpoint) bool {
	return validWorksetToken(state.Token) && strings.TrimSpace(state.Issn) != "" && state.Generation <= 1 && validTimestamp(state.FrozenAt)
}

// validCrossrefCreationBounds checks only an explicitly supplied lower creation bound.
func validCrossrefCreationBounds(state CrossrefCheckpoint) bool {
	return state.CreatedFrom == nil || *state.CreatedFrom <= state.FrozenAt && validTimestamp(*state.CreatedFrom)
}

// validCrossrefOptionalMetadata admits nil counters, filters and candidates independently.
func validCrossrefOptionalMetadata(state CrossrefCheckpoint) bool {
	return (state.RootTotal == nil || *state.RootTotal <= math.MaxInt64) && (state.UpdatedFrom == nil || validSortDate(*state.UpdatedFrom)) && (state.Candidate == nil || state.Candidate.IsValid())
}

// validCrossrefPhase applies only the constraints of the selected durable phase.
func validCrossrefPhase(state CrossrefCheckpoint) bool {
	phase := state.Phase
	switch phase.Kind {
	case "discover":
		return state.CreatedFrom == nil && state.RootTotal == nil
	case "collect":
		return validCrossrefCollectionBounds(state) && validCrossrefExpectedCount(phase) && validCrossrefCollectionCursor(phase)
	case "ready":
		return validCrossrefReadyState(state)
	case "emit":
		return validCrossrefEmissionState(state)
	}
	return false
}

// validCrossrefCollectionBounds retains partition authority, inclusive bounds and retry limits.
func validCrossrefCollectionBounds(state CrossrefCheckpoint) bool {
	phase := state.Phase
	return phase.Partition > 0 && (phase.Partition == 1 || state.RootTotal != nil) && state.CreatedFrom != nil && phase.From >= *state.CreatedFrom && phase.From <= phase.Until && phase.Until <= state.FrozenAt && phase.Retry <= 1
}

// validCrossrefExpectedCount checks supplied totals and cumulative counts before cursor constraints.
func validCrossrefExpectedCount(phase CrossrefPhase) bool {
	return phase.Expected == nil || *phase.Expected <= math.MaxInt64 && phase.Received <= *phase.Expected
}

// validCrossrefCollectionCursor distinguishes a single-second cursor from an untouched probe.
func validCrossrefCollectionCursor(phase CrossrefPhase) bool {
	if phase.Cursor != nil {
		return phase.From == phase.Until && *phase.Cursor != "" && phase.Expected != nil && *phase.Expected > 225
	}
	return phase.Received == 0
}

// validCrossrefReadyState admits a verified total with a lower creation bound or a present zero total.
func validCrossrefReadyState(state CrossrefCheckpoint) bool {
	return state.RootTotal != nil && (state.CreatedFrom != nil || *state.RootTotal == 0)
}

// validCrossrefLowerBound checks an optional inclusive emission lower date.
func validCrossrefLowerBound(phase CrossrefPhase) bool {
	return phase.Lower == nil || validSortDate(*phase.Lower) && *phase.Lower <= phase.Upper
}

// validCrossrefEmissionKey admits only the original positive rank, identity and optional sorting date.
func validCrossrefEmissionKey(after *EmissionKey) bool {
	return after == nil || after.Rank > 0 && after.Key != "" && (after.Date == "" || validSortDate(after.Date))
}

// validCrossrefEmissionState checks a verified root and the selected immutable emission window.
func validCrossrefEmissionState(state CrossrefCheckpoint) bool {
	phase := state.Phase
	return validCrossrefReadyState(state) && validSortDate(phase.Upper) && validCrossrefLowerBound(phase) && validCrossrefEmissionKey(phase.After)
}
