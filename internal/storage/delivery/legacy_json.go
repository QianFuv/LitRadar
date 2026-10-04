package delivery

import (
	"bytes"
	"encoding/json"
	"reflect"

	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
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
	if !json.Valid(data) {
		return state, &Error{Kind: "json"}
	}
	if err := decodeLegacyValue(bytes.TrimSpace(data), reflect.ValueOf(&state).Elem()); err != nil {
		return legacyState{}, err
	}
	return state, nil
}

func decodeLegacyValue(data []byte, value reflect.Value) error {
	data = bytes.TrimSpace(data)
	if value.Kind() == reflect.Pointer {
		if bytes.Equal(data, []byte("null")) {
			value.SetZero()
			return nil
		}
		value.Set(reflect.New(value.Type().Elem()))
		return decodeLegacyValue(data, value.Elem())
	}
	if len(data) == 0 || bytes.Equal(data, []byte("null")) {
		return &Error{Kind: "json"}
	}
	switch value.Kind() {
	case reflect.Struct:
		kind := value.Type()
		seen := make([]bool, value.NumField())
		fields := map[string]int{}
		for index := range value.NumField() {
			fields[kind.Field(index).Tag.Get("json")] = index
		}
		if data[0] == '[' {
			var items []json.RawMessage
			if err := json.Unmarshal(data, &items); err != nil || len(items) > value.NumField() {
				return &Error{Kind: "json"}
			}
			for index, item := range items {
				if err := decodeLegacyValue(item, value.Field(index)); err != nil {
					return err
				}
				seen[index] = true
			}
		} else if data[0] == '{' {
			decoder := json.NewDecoder(bytes.NewReader(data))
			decoder.Token()
			for decoder.More() {
				before := decoder.InputOffset()
				token, err := decoder.Token()
				if err != nil {
					return &Error{Kind: "json", cause: err}
				}
				if err := validateLegacyKey(data[before:decoder.InputOffset()]); err != nil {
					return err
				}
				var raw json.RawMessage
				if err := decoder.Decode(&raw); err != nil {
					return &Error{Kind: "json", cause: err}
				}
				index, exists := fields[token.(string)]
				if !exists {
					continue
				}
				if seen[index] {
					return &Error{Kind: "json"}
				}
				if err := decodeLegacyValue(raw, value.Field(index)); err != nil {
					return err
				}
				seen[index] = true
			}
		} else {
			return &Error{Kind: "json"}
		}
		for index, exists := range seen {
			if !exists && kind.Field(index).Tag.Get("default") != "true" {
				return &Error{Kind: "json"}
			}
		}
	case reflect.Slice:
		if data[0] != '[' {
			return &Error{Kind: "json"}
		}
		var items []json.RawMessage
		if err := json.Unmarshal(data, &items); err != nil {
			return &Error{Kind: "json", cause: err}
		}
		value.Set(reflect.MakeSlice(value.Type(), len(items), len(items)))
		for index, item := range items {
			if err := decodeLegacyValue(item, value.Index(index)); err != nil {
				return err
			}
		}
	case reflect.Map:
		if data[0] != '{' {
			return &Error{Kind: "json"}
		}
		value.Set(reflect.MakeMap(value.Type()))
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		for decoder.More() {
			before := decoder.InputOffset()
			token, err := decoder.Token()
			if err != nil {
				return &Error{Kind: "json", cause: err}
			}
			if err := validateLegacyKey(data[before:decoder.InputOffset()]); err != nil {
				return err
			}
			var raw json.RawMessage
			if err := decoder.Decode(&raw); err != nil {
				return &Error{Kind: "json", cause: err}
			}
			child := reflect.New(value.Type().Elem()).Elem()
			if err := decodeLegacyValue(raw, child); err != nil {
				return err
			}
			value.SetMapIndex(reflect.ValueOf(token.(string)), child)
		}
	case reflect.Int64, reflect.Uint64:
		number, err := sources.ParseNumber(json.Number(data))
		if err != nil {
			return &Error{Kind: "json", cause: err}
		}
		if value.Kind() == reflect.Int64 {
			number, ok := number.AsInt64()
			if !ok {
				return &Error{Kind: "json"}
			}
			value.SetInt(number)
		} else {
			number, ok := number.AsUint64()
			if !ok {
				return &Error{Kind: "json"}
			}
			value.SetUint(number)
		}
	default:
		if _, err := transport.ParseJson(data); err != nil {
			return &Error{Kind: "json", cause: err}
		}
		if err := json.Unmarshal(data, value.Addr().Interface()); err != nil {
			return &Error{Kind: "json", cause: err}
		}
	}
	return nil
}

func validateLegacyKey(data []byte) error {
	data = bytes.TrimSpace(data)
	data = bytes.TrimPrefix(data, []byte(","))
	data = bytes.TrimSpace(data)
	if _, err := transport.ParseJson(data); err != nil {
		return &Error{Kind: "json", cause: err}
	}
	return nil
}
