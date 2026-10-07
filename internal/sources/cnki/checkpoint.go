package cnki

import (
	"encoding/json"
	"fmt"
	"reflect"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

const CheckpointVersion uint32 = 2
const AnchorVersion uint32 = 1

// Anchor identifies the newest issue covered by a successfully acknowledged run.
type Anchor struct {
	Version     uint32 `json:"version"`
	YearIssueId string `json:"year_issue_id"`
}

// Checkpoint freezes the committed boundary, observed head and next page to acknowledge.
type Checkpoint struct {
	Version              uint32  `json:"version"`
	BaseAnchorIssueId    *string `json:"base_anchor_issue_id"`
	CandidateHeadIssueId string  `json:"candidate_head_issue_id"`
	CurrentIssueId       string  `json:"current_issue_id"`
	PageIndex            uint64  `json:"page_index"`
}

// UnmarshalJSON accepts the original strict struct object and positional representations.
func (anchor *Anchor) UnmarshalJSON(body []byte) error {
	var decoded Anchor
	if err := decodeState(body, []string{"version", "year_issue_id"}, &decoded); err != nil {
		return err
	}
	*anchor = decoded
	return nil
}

// UnmarshalJSON retains nullable base anchors while rejecting unknown or duplicate fields.
func (checkpoint *Checkpoint) UnmarshalJSON(body []byte) error {
	var decoded Checkpoint
	if err := decodeState(body, []string{"version", "base_anchor_issue_id", "candidate_head_issue_id", "current_issue_id", "page_index"}, &decoded); err != nil {
		return err
	}
	*checkpoint = decoded
	return nil
}
func decodeState(body []byte, names []string, target any) error {
	values, err := decodeFields(body, names, false, true)
	if err != nil {
		return err
	}
	value := reflect.ValueOf(target).Elem()
	for index, name := range names {
		raw, exists := values[name]
		if !exists {
			if value.Field(index).Kind() == reflect.Pointer {
				continue
			}
			return errFixtureJson
		}
		if err := decodeTyped(raw, value.Field(index)); err != nil {
			return err
		}
	}
	return nil
}

// Encode preserves declared field order and non-HTML-escaped compact JSON.
func (anchor Anchor) Encode() (string, error) {
	id, err := domain.Json(anchor.YearIssueId)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf(`{"version":%d,"year_issue_id":%s}`, anchor.Version, id), nil
}

// Encode emits the complete checkpoint, including an explicit null base anchor.
func (checkpoint Checkpoint) Encode() (string, error) {
	var base any
	if checkpoint.BaseAnchorIssueId != nil {
		base = *checkpoint.BaseAnchorIssueId
	}
	fields := []any{base, checkpoint.CandidateHeadIssueId, checkpoint.CurrentIssueId}
	encoded := make([][]byte, len(fields))
	for index, value := range fields {
		var err error
		encoded[index], err = domain.Json(value)
		if err != nil {
			return "", err
		}
	}
	return fmt.Sprintf(`{"version":%d,"base_anchor_issue_id":%s,"candidate_head_issue_id":%s,"current_issue_id":%s,"page_index":%d}`, checkpoint.Version, encoded[0], encoded[1], encoded[2], checkpoint.PageIndex), nil
}

// IsStableIssueId checks the opaque identifier grammar without imposing calendar semantics.
func IsStableIssueId(value string) bool {
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
