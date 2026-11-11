package delivery

import (
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

type legacyState struct {
	DbName             string            `json:"db_name"`
	Status             string            `json:"status" default:"true"`
	LastCompletedRunAt *string           `json:"last_completed_run_at" default:"true"`
	Snapshot           legacySnapshot    `json:"snapshot" default:"true"`
	Run                *legacyRun        `json:"run" default:"true"`
	DeliveryDedupe     map[string]string `json:"delivery_dedupe" default:"true"`
}

type legacySnapshot struct {
	IssueArticleCounts   map[string]int64 `json:"issue_article_counts" default:"true"`
	InpressArticleCounts map[string]int64 `json:"inpress_article_counts" default:"true"`
}

type legacyRun struct {
	RunId               string             `json:"run_id"`
	Status              string             `json:"status" default:"true"`
	PendingIssueKeys    []string           `json:"pending_issue_keys" default:"true"`
	DoneIssueKeys       []string           `json:"done_issue_keys" default:"true"`
	PendingInpressKeys  []string           `json:"pending_inpress_keys" default:"true"`
	DoneInpressKeys     []string           `json:"done_inpress_keys" default:"true"`
	DeliveredArticleIds []int64            `json:"delivered_article_ids" default:"true"`
	UserResults         []legacyUserResult `json:"user_results" default:"true"`
}

type legacyUserResult struct {
	SubscriberId      string  `json:"subscriber_id"`
	SelectedCount     uint64  `json:"selected_count" default:"true"`
	PushedCount       uint64  `json:"pushed_count" default:"true"`
	FolderSyncedCount *uint64 `json:"folder_synced_count" default:"true"`
	Status            string  `json:"status" default:"true"`
}

func decodeLegacy(data []byte) (legacyState, error) {
	var state legacyState
	if jsonobject.DecodeWithOptions(data, &state, jsonobject.Options{AllowUnknown: true}) != nil {
		return legacyState{}, &Error{Kind: "json"}
	}
	return state, nil
}

func (value *legacySnapshot) UnmarshalJSON(data []byte) error {
	type wire legacySnapshot
	var decoded wire
	if err := jsonobject.DecodeWithOptions(data, &decoded, jsonobject.Options{AllowUnknown: true}); err != nil {
		return err
	}
	*value = legacySnapshot(decoded)
	return nil
}

func (value *legacyRun) UnmarshalJSON(data []byte) error {
	type wire legacyRun
	var decoded wire
	if err := jsonobject.DecodeWithOptions(data, &decoded, jsonobject.Options{AllowUnknown: true}); err != nil {
		return err
	}
	*value = legacyRun(decoded)
	return nil
}

func (value *legacyUserResult) UnmarshalJSON(data []byte) error {
	type wire legacyUserResult
	var decoded wire
	if err := jsonobject.DecodeWithOptions(data, &decoded, jsonobject.Options{AllowUnknown: true}); err != nil {
		return err
	}
	*value = legacyUserResult(decoded)
	return nil
}
