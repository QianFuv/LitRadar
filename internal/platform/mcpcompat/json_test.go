package mcpcompat

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestStrictJsonAgainstRustReader(t *testing.T) {
	data, err := os.ReadFile("../../../tests/data/migration/json-syntax.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Hex      string
			Syntax   *string
			Envelope *string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		body, err := hex.DecodeString(scenario.Hex)
		if err != nil {
			t.Fatal(err)
		}
		actual := mcp.ValidateLitRadarBody(body)
		expected := ""
		if scenario.Syntax != nil {
			expected = *scenario.Syntax
		}
		message := ""
		if actual != nil {
			message = actual.Error()
		}
		if message != expected {
			t.Errorf("input %q: got %q want %q", body, message, expected)
		}
		expected, message = "", ""
		if scenario.Envelope != nil {
			expected = *scenario.Envelope
		}
		if err := mcp.ValidateLitRadarMessage(body); err != nil {
			message = err.Error()
		}
		if message != expected {
			t.Errorf("envelope %q: got %q want %q", body, message, expected)
		}
	}
}
