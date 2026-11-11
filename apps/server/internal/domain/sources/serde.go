//go:build go1.27

package sources

import (
	"bytes"
	"encoding/json/jsontext"
	jsonv2 "encoding/json/v2"
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

var errContractJson = errors.New("invalid canonical provider contract JSON")

func decodeContractStruct(data []byte, target any) error {
	if err := jsonobject.Decode(data, target); err != nil {
		return errContractJson
	}
	return nil
}

// UnmarshalJSON enforces required fields and typed object rules.
func (value *JournalRankings) UnmarshalJSON(data []byte) error {
	type wire JournalRankings
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalRankings(decoded)
	return nil
}

// UnmarshalJSON preserves the maintained catalog's typed object contract.
func (value *JournalCatalogEntry) UnmarshalJSON(data []byte) error {
	type wire JournalCatalogEntry
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalCatalogEntry(decoded)
	return nil
}

// MarshalJSON emits required empty collections as arrays, never null.
func (value JournalCatalogEntry) MarshalJSON() ([]byte, error) {
	type wire JournalCatalogEntry
	if value.CatalogAliases == nil {
		value.CatalogAliases = []string{}
	}
	if value.AllIssns == nil {
		value.AllIssns = []string{}
	}
	if value.TitleAliases == nil {
		value.TitleAliases = []string{}
	}
	return jsonv2.Marshal(wire(value))
}

// UnmarshalJSON preserves strict journal observation object fields.
func (value *JournalDraft) UnmarshalJSON(data []byte) error {
	type wire JournalDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = JournalDraft(decoded)
	return nil
}

// MarshalJSON retains empty observed collections as arrays.
func (value JournalDraft) MarshalJSON() ([]byte, error) {
	type wire JournalDraft
	if value.ObservedIssns == nil {
		value.ObservedIssns = []string{}
	}
	if value.ObservedTitleAliases == nil {
		value.ObservedTitleAliases = []string{}
	}
	return jsonv2.Marshal(wire(value))
}

// UnmarshalJSON rejects invalid issue field types, including integral floating-point years.
func (value *IssueDraft) UnmarshalJSON(data []byte) error {
	type wire IssueDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = IssueDraft(decoded)
	return nil
}

// UnmarshalJSON validates one required canonical author field.
func (value *ArticleAuthorDraft) UnmarshalJSON(data []byte) error {
	type wire ArticleAuthorDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ArticleAuthorDraft(decoded)
	return nil
}

// UnmarshalJSON enforces the complete typed article contract before content validation.
func (value *ArticleDraft) UnmarshalJSON(data []byte) error {
	type wire ArticleDraft
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ArticleDraft(decoded)
	return nil
}

// MarshalJSON preserves the required author and retraction collections.
func (value ArticleDraft) MarshalJSON() ([]byte, error) {
	type wire ArticleDraft
	if value.Authors == nil {
		value.Authors = []ArticleAuthorDraft{}
	}
	if value.RetractionDois == nil {
		value.RetractionDois = []string{}
	}
	return jsonv2.Marshal(wire(value))
}

// UnmarshalJSON validates nested batches before they can reach persistence.
func (value *ProviderBatch) UnmarshalJSON(data []byte) error {
	type wire ProviderBatch
	var decoded wire
	if err := decodeContractStruct(data, &decoded); err != nil {
		return err
	}
	*value = ProviderBatch(decoded)
	return nil
}

// MarshalJSON emits an empty content page using the original array shape.
func (value ProviderBatch) MarshalJSON() ([]byte, error) {
	type wire ProviderBatch
	if value.Issues == nil {
		value.Issues = []IssueDraft{}
	}
	if value.Articles == nil {
		value.Articles = []ArticleDraft{}
	}
	return jsonv2.Marshal(wire(value))
}

// UnmarshalJSON rejects synchronization modes outside the frozen enum.
func (value *IndexSyncMode) UnmarshalJSON(data []byte) error {
	var decoded string
	if jsonv2.Unmarshal(data, &decoded) != nil {
		return errContractJson
	}
	switch IndexSyncMode(decoded) {
	case Bootstrap, Incremental, FullRescan:
		*value = IndexSyncMode(decoded)
		return nil
	}
	return errContractJson
}

// UnmarshalJSON enforces variant-specific fields and required continuation checkpoints.
func (value *ProviderProgress) UnmarshalJSON(data []byte) error {
	if !jsontext.Value(data).IsValid() {
		return errContractJson
	}
	data = bytes.TrimSpace(data)
	var selector struct {
		State string `json:"state"`
	}
	if jsonv2.Unmarshal(data, &selector) != nil {
		return errContractJson
	}
	switch ProgressState(selector.State) {
	case Continue:
		var wire struct {
			State      string `json:"state"`
			Checkpoint string `json:"checkpoint"`
		}
		if err := decodeContractStruct(data, &wire); err != nil {
			return err
		}
		*value = ProviderProgress{State: Continue, Checkpoint: &wire.Checkpoint}
		return nil
	case Complete:
		var wire struct {
			State      string  `json:"state"`
			NextAnchor *string `json:"next_anchor,omitempty"`
		}
		if err := decodeContractStruct(data, &wire); err != nil {
			return err
		}
		*value = ProviderProgress{State: Complete, NextAnchor: wire.NextAnchor}
		return nil
	default:
		return errContractJson
	}
}

// MarshalJSON writes only the fields of the selected provider progress variant.
func (value ProviderProgress) MarshalJSON() ([]byte, error) {
	switch value.State {
	case Continue:
		if value.Checkpoint == nil || value.NextAnchor != nil {
			return nil, errContractJson
		}
		return jsonv2.Marshal(struct {
			State      ProgressState `json:"state"`
			Checkpoint string        `json:"checkpoint"`
		}{value.State, *value.Checkpoint})
	case Complete:
		if value.Checkpoint != nil {
			return nil, errContractJson
		}
		return jsonv2.Marshal(struct {
			State      ProgressState `json:"state"`
			NextAnchor *string       `json:"next_anchor,omitempty"`
		}{value.State, value.NextAnchor})
	default:
		return nil, errContractJson
	}
}
