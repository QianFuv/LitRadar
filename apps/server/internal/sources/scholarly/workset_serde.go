//go:build go1.27

package scholarly

import (
	"encoding/json"
	jsonv2 "encoding/json/v2"
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

var errWorksetJSON = errors.New("invalid Crossref workset metadata; restart the indexing run")

func decodeWorksetStruct(body []byte, target any) error {
	if err := jsonobject.Decode(body, target); err != nil {
		return errWorksetJSON
	}
	return nil
}

func worksetKind(body []byte) (string, error) {
	var selector struct {
		Kind string `json:"kind"`
	}
	if jsonv2.Unmarshal(body, &selector) != nil || selector.Kind == "" {
		return "", errWorksetJSON
	}
	return selector.Kind, nil
}

func decodeUnitPhase(body []byte) error {
	var value map[string]json.RawMessage
	if jsonv2.Unmarshal(body, &value) != nil || value == nil || len(value) != 1 {
		return errWorksetJSON
	}
	return nil
}

func encodeWorksetStruct(value any) ([]byte, error) { return jsonv2.Marshal(value) }
