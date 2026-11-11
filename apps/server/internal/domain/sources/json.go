//go:build go1.27

package sources

import json "encoding/json/v2"

// Json encodes typed source values with encoding/json.
func Json(value any) ([]byte, error) {
	encoded, err := json.Marshal(value, json.Deterministic(true))
	if err != nil {
		return nil, err
	}
	return encoded, nil
}
