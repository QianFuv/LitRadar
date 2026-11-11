package delivery

import (
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

// SubscriberResult is the durable summary used to reconstruct a terminal delivery without repeating effects.
type SubscriberResult struct {
	SelectedArticleIds []int64 `json:"selected_article_ids" default:"true"`
	FolderSyncedCount  uint64  `json:"folder_synced_count" default:"true"`
	MessageID          *string `json:"message_id" default:"true"`
}

// ParseSubscriberResult decodes a typed object with default values.
func ParseSubscriberResult(data []byte) (SubscriberResult, error) {
	result := SubscriberResult{SelectedArticleIds: []int64{}}
	if jsonobject.DecodeWithOptions(data, &result, jsonobject.Options{AllowUnknown: true}) != nil {
		return SubscriberResult{}, errors.New("stored subscriber result is invalid")
	}
	return result, nil
}
