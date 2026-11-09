package delivery

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
)

// SubscriberResult is the durable summary used to reconstruct a terminal delivery without repeating effects.
type SubscriberResult struct {
	SelectedArticleIds []int64 `json:"selected_article_ids" default:"true"`
	FolderSyncedCount  uint64  `json:"folder_synced_count" default:"true"`
	MessageId          *string `json:"message_id" default:"true"`
}

// ParseSubscriberResult preserves typed serde defaults, duplicate rejection and positional struct support.
func ParseSubscriberResult(data []byte) (SubscriberResult, error) {
	result := SubscriberResult{SelectedArticleIds: []int64{}}
	if !json.Valid(data) || decodeLegacyValue(bytes.TrimSpace(data), reflect.ValueOf(&result).Elem()) != nil {
		return SubscriberResult{}, errors.New("Stored subscriber result is invalid")
	}
	return result, nil
}
