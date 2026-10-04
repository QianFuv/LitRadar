package recommend

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/transport"
)

// ChangeManifest carries the original ordered, deduplicated notification inputs.
type ChangeManifest struct {
	PendingIssueKeys   []string `json:"pending_issue_keys"`
	PendingInpressKeys []string `json:"pending_inpress_keys"`
	PendingArticleIds  []int64  `json:"pending_article_ids"`
	RunId              *string  `json:"run_id"`
}

var ErrManifestJson = errors.New("Invalid change manifest JSON")

// LoadChangeManifest reads one publication without modifying it or admitting a run.
func LoadChangeManifest(filename, dbName string) (ChangeManifest, error) {
	data, err := os.ReadFile(filename)
	if err != nil {
		return ChangeManifest{}, err
	}
	return ParseChangeManifest(data, dbName)
}

// ParseChangeManifest keeps string/numeric list coercion and validation order of the original publication reader.
func ParseChangeManifest(data []byte, dbName string) (ChangeManifest, error) {
	data = bytes.TrimLeft(data, " \t\r\n\f")
	if len(data) > 0 && data[0] != '{' {
		return ChangeManifest{}, errors.New("Invalid change manifest file")
	}
	fields, err := manifestFields(data, map[string]bool{"db_name": true, "changed_issue_keys": true, "changed_inpress_journal_ids": true, "notifiable_article_ids": true, "run_id": true})
	if err != nil {
		return ChangeManifest{}, err
	}
	optional := func(name string) (*string, error) {
		value := fields[name]
		if value == nil {
			return nil, nil
		}
		text, ok := value.(string)
		if !ok {
			return nil, ErrManifestJson
		}
		return &text, nil
	}
	database, err := optional("db_name")
	if err != nil {
		return ChangeManifest{}, err
	}
	runId, err := optional("run_id")
	if err != nil {
		return ChangeManifest{}, err
	}
	if database != nil && strings.TrimSpace(*database) != "" && strings.TrimSpace(*database) != dbName {
		return ChangeManifest{}, fmt.Errorf("Change manifest database mismatch: expected %s, got %s", dbName, strings.TrimSpace(*database))
	}
	result := ChangeManifest{PendingIssueKeys: []string{}, PendingInpressKeys: []string{}, PendingArticleIds: []int64{}}
	issues, _ := fields["changed_issue_keys"].([]any)
	for _, item := range issues {
		if text, ok := item.(string); ok {
			if _, _, isValid := parseIssueKey(text); isValid {
				result.PendingIssueKeys = append(result.PendingIssueKeys, text)
			}
		}
	}
	sort.SliceStable(result.PendingIssueKeys, func(first, second int) bool {
		firstJournal, firstIssue, _ := parseIssueKey(result.PendingIssueKeys[first])
		secondJournal, secondIssue, _ := parseIssueKey(result.PendingIssueKeys[second])
		if firstJournal != secondJournal {
			return firstJournal < secondJournal
		}
		return firstIssue < secondIssue
	})
	result.PendingIssueKeys = dedupeAdjacent(result.PendingIssueKeys)
	inpress, _ := fields["changed_inpress_journal_ids"].([]any)
	for _, item := range inpress {
		if id, ok := jsonInt64(item); ok {
			result.PendingInpressKeys = append(result.PendingInpressKeys, strconv.FormatInt(id, 10))
		}
	}
	sort.SliceStable(result.PendingInpressKeys, func(first, second int) bool {
		return integerOrZero(result.PendingInpressKeys[first]) < integerOrZero(result.PendingInpressKeys[second])
	})
	result.PendingInpressKeys = dedupeAdjacent(result.PendingInpressKeys)
	articles, ok := fields["notifiable_article_ids"].([]any)
	if !ok {
		return ChangeManifest{}, errors.New("Change manifest missing notifiable_article_ids")
	}
	seen := map[int64]bool{}
	for _, item := range articles {
		if id, ok := jsonInt64(item); ok && !seen[id] {
			seen[id] = true
			result.PendingArticleIds = append(result.PendingArticleIds, id)
		}
	}
	if runId != nil {
		trimmed := strings.TrimSpace(*runId)
		if trimmed != "" {
			result.RunId = &trimmed
		}
	}
	return result, nil
}

func dedupeAdjacent(values []string) []string {
	result := values[:0]
	for _, value := range values {
		if len(result) == 0 || result[len(result)-1] != value {
			result = append(result, value)
		}
	}
	return result
}

func manifestFields(data []byte, known map[string]bool) (map[string]any, error) {
	if !json.Valid(data) {
		return nil, ErrManifestJson
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') {
		return nil, ErrManifestJson
	}
	fields := map[string]any{}
	for decoder.More() {
		start := decoder.InputOffset()
		key, err := decoder.Token()
		if err != nil {
			return nil, ErrManifestJson
		}
		rawKey := bytes.TrimSpace(bytes.TrimPrefix(bytes.TrimSpace(data[start:decoder.InputOffset()]), []byte(",")))
		if _, err := transport.ParseJson(rawKey); err != nil {
			return nil, ErrManifestJson
		}
		name, ok := key.(string)
		if !ok {
			return nil, ErrManifestJson
		}
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil {
			return nil, ErrManifestJson
		}
		if !known[name] {
			continue
		}
		if _, exists := fields[name]; exists {
			return nil, ErrManifestJson
		}
		value, err := transport.ParseJson(raw)
		if err != nil {
			return nil, ErrManifestJson
		}
		fields[name] = value
	}
	return fields, nil
}

// ParseSnapshot accepts original map defaults and positional structs while rejecting malformed stored checkpoint state.
func ParseSnapshot(data []byte) (Snapshot, error) {
	invalid := errors.New("Stored delivery checkpoint is invalid")
	data = bytes.TrimSpace(data)
	if !json.Valid(data) {
		return Snapshot{}, invalid
	}
	known := map[string]bool{"issue_article_counts": true, "inpress_article_counts": true}
	var fields map[string]any
	if len(data) > 0 && data[0] == '[' {
		var values []json.RawMessage
		if json.Unmarshal(data, &values) != nil || len(values) > 2 {
			return Snapshot{}, invalid
		}
		fields = map[string]any{}
		for index, value := range values {
			parsed, err := transport.ParseJson(value)
			if err != nil {
				return Snapshot{}, invalid
			}
			fields[[]string{"issue_article_counts", "inpress_article_counts"}[index]] = parsed
		}
	} else {
		var err error
		fields, err = manifestFields(data, known)
		if err != nil {
			return Snapshot{}, invalid
		}
	}
	rawFields := map[string]json.RawMessage{}
	if data[0] == '[' {
		var values []json.RawMessage
		json.Unmarshal(data, &values)
		for index, value := range values {
			rawFields[[]string{"issue_article_counts", "inpress_article_counts"}[index]] = value
		}
	} else {
		json.Unmarshal(data, &rawFields)
	}
	result := Snapshot{IssueArticleCounts: map[string]int64{}, InpressArticleCounts: map[string]int64{}}
	for _, field := range []struct {
		name   string
		target map[string]int64
	}{{"issue_article_counts", result.IssueArticleCounts}, {"inpress_article_counts", result.InpressArticleCounts}} {
		value, exists := fields[field.name]
		if !exists {
			continue
		}
		object, ok := value.(map[string]any)
		if !ok {
			return Snapshot{}, invalid
		}
		_ = object
		decoder := json.NewDecoder(bytes.NewReader(rawFields[field.name]))
		decoder.UseNumber()
		decoder.Token()
		for decoder.More() {
			key, err := decoder.Token()
			if err != nil {
				return Snapshot{}, invalid
			}
			var item any
			if decoder.Decode(&item) != nil {
				return Snapshot{}, invalid
			}
			if _, isString := item.(string); isString {
				return Snapshot{}, invalid
			}
			number, ok := jsonInt64(item)
			if !ok {
				return Snapshot{}, invalid
			}
			field.target[key.(string)] = number
		}
	}
	return result, nil
}
