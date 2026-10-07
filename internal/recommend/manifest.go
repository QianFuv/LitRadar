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
	database, err := manifestOptionalString(fields, "db_name")
	if err != nil {
		return ChangeManifest{}, err
	}
	runId, err := manifestOptionalString(fields, "run_id")
	if err != nil {
		return ChangeManifest{}, err
	}
	if err := validateManifestDatabase(database, dbName); err != nil {
		return ChangeManifest{}, err
	}
	result := ChangeManifest{PendingIssueKeys: []string{}, PendingInpressKeys: []string{}, PendingArticleIds: []int64{}}
	result.PendingIssueKeys = manifestIssueKeys(fields["changed_issue_keys"])
	result.PendingInpressKeys = manifestInpressKeys(fields["changed_inpress_journal_ids"])
	result.PendingArticleIds, err = manifestArticleIds(fields["notifiable_article_ids"])
	if err != nil {
		return ChangeManifest{}, err
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
		name, raw, err := readManifestField(data, decoder)
		if err != nil {
			return nil, err
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
	fields, err := snapshotParsedFields(data, invalid)
	if err != nil {
		return Snapshot{}, err
	}
	rawFields := snapshotRawFields(data)
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
		if err := decodeSnapshotCounts(rawFields[field.name], field.target, invalid); err != nil {
			return Snapshot{}, err
		}
	}
	return result, nil
}

func manifestOptionalString(fields map[string]any, name string) (*string, error) {
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

func manifestIssueKeys(value any) []string {
	keys := []string{}
	issues, _ := value.([]any)
	for _, item := range issues {
		if text, ok := item.(string); ok {
			if _, _, isValid := parseIssueKey(text); isValid {
				keys = append(keys, text)
			}
		}
	}
	sort.SliceStable(keys, func(first, second int) bool {
		firstJournal, firstIssue, _ := parseIssueKey(keys[first])
		secondJournal, secondIssue, _ := parseIssueKey(keys[second])
		if firstJournal != secondJournal {
			return firstJournal < secondJournal
		}
		return firstIssue < secondIssue
	})
	keys = dedupeAdjacent(keys)
	return keys
}

func manifestInpressKeys(value any) []string {
	keys := []string{}
	inpress, _ := value.([]any)
	for _, item := range inpress {
		if id, ok := jsonInt64(item); ok {
			keys = append(keys, strconv.FormatInt(id, 10))
		}
	}
	sort.SliceStable(keys, func(first, second int) bool {
		return integerOrZero(keys[first]) < integerOrZero(keys[second])
	})
	keys = dedupeAdjacent(keys)
	return keys
}

func manifestArticleIds(value any) ([]int64, error) {
	ids := []int64{}
	articles, ok := value.([]any)
	if !ok {
		return nil, errors.New("Change manifest missing notifiable_article_ids")
	}
	seen := map[int64]bool{}
	for _, item := range articles {
		if id, ok := jsonInt64(item); ok && !seen[id] {
			seen[id] = true
			ids = append(ids, id)
		}
	}
	return ids, nil
}

func readManifestField(data []byte, decoder *json.Decoder) (string, json.RawMessage, error) {
	start := decoder.InputOffset()
	key, err := decoder.Token()
	if err != nil {
		return "", nil, ErrManifestJson
	}
	rawKey := bytes.TrimSpace(bytes.TrimPrefix(bytes.TrimSpace(data[start:decoder.InputOffset()]), []byte(",")))
	if _, err := transport.ParseJson(rawKey); err != nil {
		return "", nil, ErrManifestJson
	}
	name, ok := key.(string)
	if !ok {
		return "", nil, ErrManifestJson
	}
	var raw json.RawMessage
	if decoder.Decode(&raw) != nil {
		return "", nil, ErrManifestJson
	}
	return name, raw, nil
}

func snapshotParsedFields(data []byte, invalid error) (map[string]any, error) {
	known := map[string]bool{"issue_article_counts": true, "inpress_article_counts": true}
	var fields map[string]any
	if len(data) > 0 && data[0] == '[' {
		var values []json.RawMessage
		if json.Unmarshal(data, &values) != nil || len(values) > 2 {
			return nil, invalid
		}
		fields = map[string]any{}
		for index, value := range values {
			parsed, err := transport.ParseJson(value)
			if err != nil {
				return nil, invalid
			}
			fields[[]string{"issue_article_counts", "inpress_article_counts"}[index]] = parsed
		}
	} else {
		var err error
		fields, err = manifestFields(data, known)
		if err != nil {
			return nil, invalid
		}
	}
	return fields, nil
}

func snapshotRawFields(data []byte) map[string]json.RawMessage {
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
	return rawFields
}

func decodeSnapshotCounts(raw json.RawMessage, target map[string]int64, invalid error) error {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	decoder.Token()
	for decoder.More() {
		key, err := decoder.Token()
		if err != nil {
			return invalid
		}
		var item any
		if decoder.Decode(&item) != nil {
			return invalid
		}
		if _, isString := item.(string); isString {
			return invalid
		}
		number, ok := jsonInt64(item)
		if !ok {
			return invalid
		}
		target[key.(string)] = number
	}
	return nil
}

func validateManifestDatabase(database *string, dbName string) error {
	if database != nil && strings.TrimSpace(*database) != "" && strings.TrimSpace(*database) != dbName {
		return fmt.Errorf("Change manifest database mismatch: expected %s, got %s", dbName, strings.TrimSpace(*database))
	}
	return nil
}
