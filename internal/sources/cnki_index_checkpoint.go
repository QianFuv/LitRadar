package sources

import (
	"encoding/json"
	"strings"

	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
)

func rejectCnkiOpaqueSecrets(raw, kind string) error {
	lowered := asciiLowerSource(raw)
	for _, forbidden := range []string{"captcha", "secretkey", "pointjson", "jfbym", "session", "cookie", "token", "http://", "https://", "url"} {
		if strings.Contains(lowered, forbidden) {
			return accessFailure(provider.InvalidResponse, "domestic CNKI "+kind+" must not contain session or transport fields")
		}
	}
	return nil
}

func decodeCnkiAnchor(raw string) (*cnki.Anchor, error) {
	if err := rejectCnkiOpaqueSecrets(raw, "anchor"); err != nil {
		return nil, err
	}
	var anchor cnki.Anchor
	if json.Unmarshal([]byte(raw), &anchor) != nil {
		return nil, accessFailure(provider.InvalidResponse, "domestic CNKI anchor is invalid")
	}
	if anchor.Version != cnki.AnchorVersion || !cnki.IsStableIssueId(anchor.YearIssueId) {
		return nil, accessFailure(provider.InvalidResponse, "domestic CNKI anchor version or issue id is invalid")
	}
	return &anchor, nil
}

func decodeCnkiCheckpoint(raw string) (*cnki.Checkpoint, error) {
	if err := rejectCnkiOpaqueSecrets(raw, "checkpoint"); err != nil {
		return nil, err
	}
	var checkpoint cnki.Checkpoint
	if json.Unmarshal([]byte(raw), &checkpoint) != nil {
		return nil, accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint is invalid")
	}
	return &checkpoint, nil
}

func missingCnkiCheckpointIssue() error {
	return accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint issue is missing; reset the disposable control database")
}
