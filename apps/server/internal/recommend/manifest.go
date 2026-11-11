package recommend

import (
	"bytes"
	"cmp"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
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
	root, name, err := rootfile.Parent(filename)
	if err != nil {
		return ChangeManifest{}, err
	}
	defer func() { _ = root.Close() }()
	data, err := root.ReadFile(name)
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

// ParseSnapshot accepts object snapshots with default empty counts while rejecting malformed stored checkpoint state.
func ParseSnapshot(data []byte) (Snapshot, error) {
	invalid := errors.New("stored delivery checkpoint is invalid or obsolete; expected a typed object")
	var wire struct {
		Issue   map[string]*int64 `json:"issue_article_counts" default:"true"`
		Inpress map[string]*int64 `json:"inpress_article_counts" default:"true"`
	}
	if jsonobject.DecodeWithOptions(data, &wire, jsonobject.Options{AllowUnknown: true}) != nil {
		return Snapshot{}, invalid
	}
	result := Snapshot{IssueArticleCounts: map[string]int64{}, InpressArticleCounts: map[string]int64{}}
	for _, group := range []struct {
		source map[string]*int64
		target map[string]int64
	}{{wire.Issue, result.IssueArticleCounts}, {wire.Inpress, result.InpressArticleCounts}} {
		for key, count := range group.source {
			if count == nil {
				return Snapshot{}, invalid
			}
			group.target[key] = *count
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
	slices.SortStableFunc(keys, compareIssueKeys)
	keys = slices.Compact(keys)
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
	slices.SortStableFunc(keys, func(first, second string) int { return cmp.Compare(integerOrZero(first), integerOrZero(second)) })
	keys = slices.Compact(keys)
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

func validateManifestDatabase(database *string, dbName string) error {
	if database != nil && strings.TrimSpace(*database) != "" && strings.TrimSpace(*database) != dbName {
		return fmt.Errorf("Change manifest database mismatch: expected %s, got %s", dbName, strings.TrimSpace(*database))
	}
	return nil
}

// compareIssueKeys orders journal and issue identifiers numerically without changing equal-key order.
func compareIssueKeys(first, second string) int {
	firstJournal, firstIssue, _ := parseIssueKey(first)
	secondJournal, secondIssue, _ := parseIssueKey(second)
	return cmp.Or(cmp.Compare(firstJournal, secondJournal), cmp.Compare(firstIssue, secondIssue))
}
