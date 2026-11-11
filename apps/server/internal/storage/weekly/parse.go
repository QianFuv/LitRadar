//go:build go1.27

package weekly

import (
	"bytes"
	"encoding/json"
	jsonv2 "encoding/json/v2"
	"errors"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
)

// ErrManifestJson distinguishes invalid typed publications from intentionally ignored ones.
var ErrManifestJson = errors.New("invalid weekly manifest JSON; expected an object with an RFC3339Nano generated_at; rerun indexing to rebuild the publication")

// Manifest retains source order and untrimmed publication identity before optional delivery enrichment.
type Manifest struct {
	DbName      string
	RunId       *string
	GeneratedAt Timestamp
	ArticleIds  []int64
}

// ParseManifest decodes object publications and requires RFC3339Nano generation times.
func ParseManifest(data []byte) (*Manifest, error) {
	var wire struct {
		DbName      *string         `json:"db_name"`
		GeneratedAt *string         `json:"generated_at"`
		RunId       *string         `json:"run_id"`
		ArticleIds  json.RawMessage `json:"notifiable_article_ids"`
	}
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' || jsonv2.Unmarshal(data, &wire) != nil {
		return nil, ErrManifestJson
	}
	if wire.DbName == nil {
		return nil, nil
	}
	database := config.NormalizeDatabaseName(*wire.DbName)
	if database == "" {
		return nil, nil
	}
	ids, err := decodeManifestArticleIds(wire.ArticleIds)
	if err != nil {
		return nil, err
	}
	if len(ids) == 0 {
		return nil, nil
	}
	if wire.GeneratedAt == nil {
		return nil, ErrManifestJson
	}
	stamp, ok := ParseTimestamp(*wire.GeneratedAt)
	if !ok {
		return nil, ErrManifestJson
	}
	return &Manifest{database, wire.RunId, stamp, ids}, nil
}

// manifestArticleId stores admitted integer tokens without retaining each element's encoded JSON.
type manifestArticleId struct {
	value     int64
	isInteger bool
}

// UnmarshalJSON ignores non-integer values and negative zero while preserving valid signed integer spelling.
func (identifier *manifestArticleId) UnmarshalJSON(value []byte) error {
	*identifier = manifestArticleId{}
	raw := strings.TrimSpace(string(value))
	if raw == "-0" || strings.ContainsAny(raw, ".eE") {
		return nil
	}
	parsed, err := strconv.ParseInt(raw, 10, 64)
	if err == nil {
		identifier.value, identifier.isInteger = parsed, true
	}
	return nil
}

// decodeManifestArticleIds retains distinct signed integers in source order without decimal coercion.
func decodeManifestArticleIds(value json.RawMessage) ([]int64, error) {
	ids := []int64{}
	seen := map[int64]bool{}
	if bytes.HasPrefix(bytes.TrimSpace(value), []byte("[")) {
		var values []manifestArticleId
		if json.Unmarshal(value, &values) != nil {
			return nil, ErrManifestJson
		}
		for _, value := range values {
			if !value.isInteger || seen[value.value] {
				continue
			}
			seen[value.value] = true
			ids = append(ids, value.value)
		}
	}
	return ids, nil
}
