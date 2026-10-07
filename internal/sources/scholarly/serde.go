package scholarly

import (
	"bytes"
	"encoding/json"
	"errors"

	"github.com/QianFuv/LitRadar/internal/transport"
)

var errSourceConfig = errors.New("invalid scholarly configuration JSON")
var fixtureFields = []string{"crossref_status", "crossref_works", "crossref_work_pages", "openalex_source_by_issns", "openalex_source_by_title", "openalex_source_works", "openalex_source_work_pages", "openalex_source_works_plan_restricted", "openalex_source_works_plan_restricted_after_page", "openalex_source_works_status", "openalex_by_doi", "semantic_scholar_status", "semantic_scholar_error", "semantic_scholar_by_doi"}
var liveFields = []string{"timeout_seconds", "openalex_api_keys", "semantic_scholar_api_keys", "crossref_mailtos", "semantic_scholar_worker_id", "semantic_scholar_process_count", "semantic_scholar_base_interval_ms", "schedule_epoch_unix_millis"}

func sourceConfigFields(body []byte, names []string, hasDefaults bool) (map[string]json.RawMessage, error) {
	if !json.Valid(body) {
		return nil, errSourceConfig
	}
	body = bytes.TrimSpace(body)
	values := map[string]json.RawMessage{}
	known := map[string]bool{}
	for _, name := range names {
		known[name] = true
	}
	if body[0] == '[' {
		if err := decodeSourceSequence(body, names, hasDefaults, values); err != nil {
			return nil, err
		}
	} else if body[0] == '{' {
		if err := decodeSourceObject(body, known, values); err != nil {
			return nil, err
		}
		if !hasDefaults && len(values) != len(names) {
			return nil, errSourceConfig
		}
	} else {
		return nil, errSourceConfig
	}
	if err := validateSourceValues(values); err != nil {
		return nil, err
	}
	return values, nil
}

// validateSourceValues checks retained nested values with the source JSON numeric grammar.
func validateSourceValues(values map[string]json.RawMessage) error {
	for _, raw := range values {
		nested := append([]byte{'['}, raw...)
		nested = append(nested, ']')
		if _, err := transport.ParseJson(nested); err != nil {
			return errSourceConfig
		}
	}
	return nil
}

// UnmarshalJSON preserves defaulted fixture sequences and source numeric variants.
func (data *FixtureData) UnmarshalJSON(body []byte) error {
	values, err := sourceConfigFields(body, fixtureFields, true)
	if err != nil {
		return err
	}
	for _, key := range []string{"crossref_works", "crossref_work_pages", "openalex_source_works", "openalex_source_work_pages", "openalex_source_works_plan_restricted", "openalex_by_doi", "semantic_scholar_by_doi"} {
		if bytes.Equal(bytes.TrimSpace(values[key]), []byte("null")) {
			return errSourceConfig
		}
	}
	if err := validateFixturePages(values); err != nil {
		return err
	}
	encoded, err := json.Marshal(values)
	if err != nil {
		return errSourceConfig
	}
	type plainFixture FixtureData
	var decoded plainFixture
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	if err := decoder.Decode(&decoded); err != nil {
		return errSourceConfig
	}
	*data = FixtureData(decoded)
	return nil
}

// UnmarshalJSON requires the original complete worker bootstrap shape and strict pool entries.
func (config *LiveConfig) UnmarshalJSON(body []byte) error {
	values, err := sourceConfigFields(body, liveFields, false)
	if err != nil {
		return err
	}
	for _, raw := range values {
		if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return errSourceConfig
		}
	}
	if err := validateSourcePools(values); err != nil {
		return err
	}
	encoded, err := json.Marshal(values)
	if err != nil {
		return errSourceConfig
	}
	type plainConfig LiveConfig
	var decoded plainConfig
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		return errSourceConfig
	}
	*config = LiveConfig(decoded)
	return nil
}

// decodeSourceSequence retains positional arity and default admission.
func decodeSourceSequence(body []byte, names []string, hasDefaults bool, values map[string]json.RawMessage) error {
	var sequence []json.RawMessage
	if err := json.Unmarshal(body, &sequence); err != nil || len(sequence) > len(names) || !hasDefaults && len(sequence) != len(names) {
		return errSourceConfig
	}
	for index, raw := range sequence {
		values[names[index]] = raw
	}
	return nil
}

// decodeSourceObject validates raw keys and rejects duplicate known fields while retaining unknown-field handling.
func decodeSourceObject(body []byte, known map[string]bool, values map[string]json.RawMessage) error {
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.Token()
	for decoder.More() {
		before := decoder.InputOffset()
		name, err := decoder.Token()
		if err != nil {
			return errSourceConfig
		}
		key := name.(string)
		rawName := bytes.TrimSpace(body[before:decoder.InputOffset()])
		rawName = bytes.TrimSpace(bytes.TrimPrefix(rawName, []byte(",")))
		if _, err := transport.ParseJson(rawName); err != nil {
			return errSourceConfig
		}
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			return errSourceConfig
		}
		if !known[key] {
			continue
		}
		if _, exists := values[key]; exists {
			return errSourceConfig
		}
		values[key] = raw
	}
	return nil
}

// validateFixturePages rejects null fixture pages without changing page decoding order.
func validateFixturePages(values map[string]json.RawMessage) error {
	for _, key := range []string{"crossref_work_pages", "openalex_source_work_pages"} {
		if raw, exists := values[key]; exists {
			var pages []json.RawMessage
			if err := json.Unmarshal(raw, &pages); err != nil {
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

// validateSourcePools checks the three source pools contain only string entries.
func validateSourcePools(values map[string]json.RawMessage) error {
	for _, name := range []string{"openalex_api_keys", "semantic_scholar_api_keys", "crossref_mailtos"} {
		var items []json.RawMessage
		if err := json.Unmarshal(values[name], &items); err != nil {
			return errSourceConfig
		}
		for _, item := range items {
			if len(bytes.TrimSpace(item)) == 0 || bytes.TrimSpace(item)[0] != '"' {
				return errSourceConfig
			}
		}
	}
	return nil
}
