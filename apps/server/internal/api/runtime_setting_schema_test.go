package api

import (
	"encoding/json"
	"testing"
)

// TestRuntimeSettingContentSchemas verifies named types and string annotations in the published document.
func TestRuntimeSettingContentSchemas(t *testing.T) {
	encoded, err := GenerateOpenAPI()
	if err != nil {
		t.Fatal(err)
	}
	var document struct {
		Components struct {
			Schemas map[string]struct {
				Type                 string          `json:"type"`
				Properties           map[string]any  `json:"properties"`
				AdditionalProperties json.RawMessage `json:"additionalProperties"`
				AllOf                []struct {
					If struct {
						Properties map[string]struct {
							Const string `json:"const"`
						} `json:"properties"`
					} `json:"if"`
					Then struct {
						Properties map[string]map[string]any `json:"properties"`
					} `json:"then"`
				} `json:"allOf"`
			} `json:"schemas"`
		} `json:"components"`
	}
	if err := json.Unmarshal(encoded, &document); err != nil {
		t.Fatal(err)
	}
	schemas := document.Components.Schemas
	for _, name := range []string{"ProviderOrders", "IndexProviderRoutes", "ProviderProxyPolicy"} {
		if schemas[name].Type != "object" {
			t.Fatal("missing named setting structure", name)
		}
	}
	if schemas["ProviderOrders"].Properties["default"] == nil || schemas["ProviderOrders"].Properties["catalogs"] == nil {
		t.Fatal("setting structures no longer describe provider orders, routes and proxy flags")
	}
	for name, kind := range map[string]string{"IndexProviderRoutes": "string", "ProviderProxyPolicy": "boolean"} {
		var property map[string]any
		if err := json.Unmarshal(schemas[name].AdditionalProperties, &property); err != nil || property["type"] != kind {
			t.Fatal("invalid provider mapping value type", name, property, err)
		}
	}
	expected := map[string]string{
		"article_abstract_provider_orders": "ProviderOrders",
		"article_fulltext_provider_orders": "ProviderOrders",
		"index_provider_routes":            "IndexProviderRoutes",
		"provider_proxy_policy":            "ProviderProxyPolicy",
	}
	values := schemas["RuntimeSettingsUpdate"].Properties["values"].(map[string]any)
	if values["additionalProperties"].(map[string]any)["type"] == nil {
		t.Fatal("generic runtime setting updates were removed")
	}
	properties := values["properties"].(map[string]any)
	for field, name := range expected {
		property := properties[field].(map[string]any)
		assertSettingContent(t, property, name)
		kinds, ok := property["type"].([]any)
		if !ok || len(kinds) != 2 || kinds[0] != "string" || kinds[1] != "null" {
			t.Fatal("setting updates must remain nullable strings", field, property)
		}
	}
	conditions := schemas["Info"].AllOf
	if len(conditions) != len(expected) {
		t.Fatal("missing field-specific setting response annotations", conditions)
	}
	for _, condition := range conditions {
		field := condition.If.Properties["field"].Const
		property := condition.Then.Properties["value"]
		assertSettingContent(t, property, expected[field])
		if property["type"] != "string" {
			t.Fatal("setting responses must remain strings", field, property)
		}
	}
	second, err := GenerateOpenAPI()
	if err != nil || string(second) != string(encoded) {
		t.Fatal("runtime setting annotations are nondeterministic", err)
	}
}

// assertSettingContent requires an encoded JSON annotation referencing the named structure.
func assertSettingContent(t *testing.T, property map[string]any, name string) {
	t.Helper()
	if name == "" || property["contentMediaType"] != "application/json" || property["contentSchema"].(map[string]any)["$ref"] != "#/components/schemas/"+name {
		t.Fatal("invalid JSON string content annotation", name, property)
	}
}
