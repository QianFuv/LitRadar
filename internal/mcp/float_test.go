package mcp

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/favorites"
)

func TestToolTextPreservesOriginalFloatingPointSpelling(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/api/mcp-float-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		ExporterSha256 string `json:"exporter_sha256"`
		Cases          []struct {
			Folders []favorites.Folder
			Text    string
		}
	}
	if err = json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	exporter, err := os.ReadFile("../../tests/migration/api/export-mcp.mjs")
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(exporter)
	if hex.EncodeToString(digest[:]) != corpus.ExporterSha256 || len(corpus.Cases) != 3 {
		t.Fatal("stale/incomplete float observations")
	}
	for _, scenario := range corpus.Cases {
		actual, err := encodeToolPayload(scenario.Folders)
		if err != nil || actual != scenario.Text {
			t.Errorf("got %s (%v); want %s", actual, err, scenario.Text)
		}
	}
}
