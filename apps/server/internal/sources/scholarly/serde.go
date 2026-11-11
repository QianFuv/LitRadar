//go:build go1.27

package scholarly

import (
	"bytes"
	"encoding/json"
	jsonv2 "encoding/json/v2"
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

var errSourceConfig = errors.New("invalid scholarly configuration JSON")

// UnmarshalJSON decodes object fixtures and retains exact upstream numeric tokens.
func (data *FixtureData) UnmarshalJSON(body []byte) error {
	var values map[string]json.RawMessage
	if jsonv2.Unmarshal(body, &values) != nil || values == nil {
		return errSourceConfig
	}
	for _, name := range []string{"crossref_works", "crossref_work_pages", "openalex_source_works", "openalex_source_work_pages", "openalex_source_works_plan_restricted", "openalex_by_doi", "semantic_scholar_by_doi"} {
		if bytes.Equal(bytes.TrimSpace(values[name]), []byte("null")) {
			return errSourceConfig
		}
	}
	if err := validateFixturePages(values); err != nil {
		return err
	}
	type wire FixtureData
	var decoded wire
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	if decoder.Decode(&decoded) != nil {
		return errSourceConfig
	}
	*data = FixtureData(decoded)
	return nil
}

// UnmarshalJSON requires a complete typed worker bootstrap object.
func (config *LiveConfig) UnmarshalJSON(body []byte) error {
	type wire LiveConfig
	var decoded wire
	if jsonobject.Decode(body, &decoded) != nil {
		return errSourceConfig
	}
	*config = LiveConfig(decoded)
	return nil
}

// validateFixturePages rejects null fixture pages without changing page decoding order.
func validateFixturePages(values map[string]json.RawMessage) error {
	for _, key := range []string{"crossref_work_pages", "openalex_source_work_pages"} {
		if raw, exists := values[key]; exists {
			var pages []json.RawMessage
			if err := jsonv2.Unmarshal(raw, &pages); err != nil {
				return errSourceConfig
			}
			for _, page := range pages {
				if bytes.Equal(bytes.TrimSpace(page), []byte("null")) {
					return errSourceConfig
				}
			}
		}
	}
	return nil
}
