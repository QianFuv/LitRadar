package weekly

import (
	"bytes"
	"encoding/json"
	"errors"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// ErrManifestJson distinguishes invalid typed publications from intentionally ignored ones.
var ErrManifestJson = errors.New("invalid weekly manifest JSON")

// Manifest retains source order and untrimmed publication identity before optional delivery enrichment.
type Manifest struct {
	DbName      string
	RunId       *string
	GeneratedAt Timestamp
	ArticleIds  []int64
}

// ParseManifest distinguishes malformed input from a valid but non-notifiable publication.
func ParseManifest(data []byte) (*Manifest, error) {
	if !json.Valid(data) {
		return nil, ErrManifestJson
	}
	fields, err := decodeManifestFields(data)
	if err != nil {
		return nil, err
	}
	optional, err := decodeManifestOptionalStrings(fields)
	if err != nil {
		return nil, err
	}
	if optional[0] == nil {
		return nil, nil
	}
	database := config.NormalizeDatabaseName(*optional[0])
	if database == "" {
		return nil, nil
	}
	ids, err := decodeManifestArticleIds(fields["notifiable_article_ids"])
	if err != nil {
		return nil, err
	}
	if len(ids) == 0 {
		return nil, nil
	}
	stamp, ok := manifestTimestamp(optional)
	if !ok {
		return nil, nil
	}
	return &Manifest{database, optional[2], stamp, ids}, nil
}

// manifestFieldNames retains positional array field order.
func manifestFieldNames() []string {
	return []string{"db_name", "generated_at", "run_id", "notifiable_article_ids"}
}

// isManifestField recognizes only source identity and notification membership fields.
func isManifestField(name string) bool {
	return name == "db_name" || name == "generated_at" || name == "run_id" || name == "notifiable_article_ids"
}

// decodeManifestFields preserves strict keys and duplicate recognized-field rejection.
func decodeManifestFields(data []byte) (map[string]json.RawMessage, error) {
	fields := map[string]json.RawMessage{}
	decoder := json.NewDecoder(bytes.NewReader(data))
	opening, err := decoder.Token()
	if err != nil {
		return nil, ErrManifestJson
	}
	names := manifestFieldNames()
	switch opening {
	case json.Delim('['):
		var values []json.RawMessage
		if json.Unmarshal(data, &values) != nil || len(values) < 3 || len(values) > 4 {
			return nil, ErrManifestJson
		}
		for index, value := range values {
			fields[names[index]] = value
		}
	case json.Delim('{'):
		if err := decodeManifestObject(data, decoder, fields); err != nil {
			return nil, err
		}
	default:
		return nil, ErrManifestJson
	}
	return fields, nil
}

// decodeManifestObject reads every key while ignoring unknown field values.
func decodeManifestObject(data []byte, decoder *json.Decoder, fields map[string]json.RawMessage) error {
	for decoder.More() {
		start := decoder.InputOffset()
		key, err := decoder.Token()
		if err != nil {
			return ErrManifestJson
		}
		name, ok := key.(string)
		if !ok {
			return ErrManifestJson
		}
		rawKey := bytes.TrimSpace(data[start:decoder.InputOffset()])
		rawKey = bytes.TrimSpace(bytes.TrimPrefix(rawKey, []byte(",")))
		if !jsonvalue.ValidJson(string(rawKey)) {
			return ErrManifestJson
		}
		var value json.RawMessage
		if decoder.Decode(&value) != nil {
			return ErrManifestJson
		}
		if !isManifestField(name) {
			continue
		}
		if _, exists := fields[name]; exists {
			return ErrManifestJson
		}
		fields[name] = value
	}
	return nil
}

// decodeManifestOptionalStrings validates retained JSON before decoding the three optional strings.
func decodeManifestOptionalStrings(fields map[string]json.RawMessage) ([]*string, error) {
	optional := make([]*string, 3)
	names := manifestFieldNames()
	for _, value := range fields {
		if !jsonvalue.ValidJson("[" + string(value) + "]") {
			return nil, ErrManifestJson
		}
	}
	for index, name := range names[:3] {
		if value, exists := fields[name]; exists && !bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
			var text string
			if json.Unmarshal(value, &text) != nil {
				return nil, ErrManifestJson
			}
			optional[index] = &text
		}
	}
	return optional, nil
}

// decodeManifestArticleIds retains distinct signed integers in source order without decimal coercion.
func decodeManifestArticleIds(value json.RawMessage) ([]int64, error) {
	ids := []int64{}
	seen := map[int64]bool{}
	if bytes.HasPrefix(bytes.TrimSpace(value), []byte("[")) {
		var values []json.RawMessage
		if json.Unmarshal(value, &values) != nil {
			return nil, ErrManifestJson
		}
		for _, value := range values {
			raw := strings.TrimSpace(string(value))
			if raw == "-0" || strings.ContainsAny(raw, ".eE") {
				continue
			}
			id, err := strconv.ParseInt(raw, 10, 64)
			if err != nil || seen[id] {
				continue
			}
			seen[id] = true
			ids = append(ids, id)
		}
	}
	return ids, nil
}

// manifestTimestamp falls back to run identity only when generated time is absent.
func manifestTimestamp(optional []*string) (Timestamp, bool) {
	date := optional[1]
	if date == nil {
		date = optional[2]
	}
	if date == nil {
		return Timestamp{}, false
	}
	return parseManifestTime(*date)
}
