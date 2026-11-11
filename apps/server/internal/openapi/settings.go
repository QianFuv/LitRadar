package openapi

import (
	"maps"
	"reflect"
	"slices"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/danielgtaylor/huma/v2"
)

var runtimeSettingContentSchemas = map[string]string{
	"article_abstract_provider_orders": "ProviderOrders",
	"article_fulltext_provider_orders": "ProviderOrders",
	"index_provider_routes":            "IndexProviderRoutes",
	"provider_proxy_policy":            "ProviderProxyPolicy",
}

// registerRuntimeSettingSchemas exposes structures encoded inside setting strings.
func registerRuntimeSettingSchemas(registry huma.Registry) {
	registry.Schema(reflect.TypeFor[settings.ProviderOrders](), true, "")
	registry.Map()["IndexProviderRoutes"] = registry.Schema(reflect.TypeFor[settings.IndexProviderRoutes](), false, "")
	registry.Map()["ProviderProxyPolicy"] = registry.Schema(reflect.TypeFor[settings.ProviderProxyPolicy](), false, "")
	orders := registry.Map()["ProviderOrders"]
	orders.Properties["catalogs"].AdditionalProperties.(*huma.Schema).Nullable = false
}

// runtimeSettingContent describes a JSON document carried as a string, not an object.
func runtimeSettingContent(name string, isNullable bool) *huma.Schema {
	return &huma.Schema{
		Type:        huma.TypeString,
		Nullable:    isNullable,
		Description: "JSON-encoded " + name + "; decode the string to access the named structure.",
		Extensions: map[string]any{
			"contentMediaType": "application/json",
			"contentSchema":    &huma.Schema{Ref: "#/components/schemas/" + name},
		},
	}
}

// DescribeRuntimeSettingValues annotates known keys without changing the string/null update contract.
func DescribeRuntimeSettingValues(schema *huma.Schema) {
	schema.Properties = map[string]*huma.Schema{}
	for field, name := range runtimeSettingContentSchemas {
		schema.Properties[field] = runtimeSettingContent(name, true)
	}
}

// describeRuntimeSettingInfo selects the decoded value structure using the setting field name.
func describeRuntimeSettingInfo(schema *huma.Schema) {
	for _, field := range slices.Sorted(maps.Keys(runtimeSettingContentSchemas)) {
		name := runtimeSettingContentSchemas[field]
		schema.AllOf = append(schema.AllOf, &huma.Schema{Extensions: map[string]any{
			"if": &huma.Schema{
				Properties: map[string]*huma.Schema{"field": {Const: field}},
				Required:   []string{"field"},
			},
			"then": &huma.Schema{Properties: map[string]*huma.Schema{"value": runtimeSettingContent(name, false)}},
		}})
	}
}
