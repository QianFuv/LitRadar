//go:build go1.27

package cnki

import (
	"encoding/json"
	jsonv2 "encoding/json/v2"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

const CheckpointVersion uint32 = 2
const AnchorVersion uint32 = 1

// Anchor identifies the newest issue covered by a successfully acknowledged run.
type Anchor struct {
	Version     uint32 `json:"version"`
	YearIssueID string `json:"year_issue_id"`
}

// Checkpoint freezes the committed boundary, observed head and next page to acknowledge.
type Checkpoint struct {
	Version              uint32  `json:"version"`
	BaseAnchorIssueID    *string `json:"base_anchor_issue_id"`
	CandidateHeadIssueID string  `json:"candidate_head_issue_id"`
	CurrentIssueID       string  `json:"current_issue_id"`
	PageIndex            uint64  `json:"page_index"`
}

// UnmarshalJSON validates a typed anchor object before replacing the receiver.
func (anchor *Anchor) UnmarshalJSON(body []byte) error {
	type wire Anchor
	var decoded wire
	if err := jsonobject.Decode(body, &decoded); err != nil {
		return err
	}
	*anchor = Anchor(decoded)
	return nil
}

// UnmarshalJSON validates a typed checkpoint object before replacing the receiver.
func (checkpoint *Checkpoint) UnmarshalJSON(body []byte) error {
	type wire Checkpoint
	var decoded wire
	if err := jsonobject.Decode(body, &decoded); err != nil {
		return err
	}
	*checkpoint = Checkpoint(decoded)
	return nil
}

// Encode serializes the typed anchor with encoding/json.
func (anchor Anchor) Encode() (string, error) {
	encoded, err := jsonv2.Marshal(anchor)
	return string(encoded), err
}

// Encode serializes the typed checkpoint with encoding/json.
func (checkpoint Checkpoint) Encode() (string, error) {
	encoded, err := jsonv2.Marshal(checkpoint)
	return string(encoded), err
}

// IsStableIssueID checks the opaque identifier grammar without imposing calendar semantics.
func IsStableIssueID(value string) bool {
	if len(value) == 0 || len(value) > 64 {
		return false
	}
	for _, character := range []byte(value) {
		if !isStableIssueCharacter(character) {
			return false
		}
	}
	return true
}

var _ json.Unmarshaler = (*Anchor)(nil)
var _ json.Unmarshaler = (*Checkpoint)(nil)

// isStableIssueCharacter admits the original ASCII identifier alphabet.
func isStableIssueCharacter(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '-' || character == '_' || character == '.'
}
