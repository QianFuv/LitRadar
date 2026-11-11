package openapi

import (
	"encoding/json"
	"maps"
	"reflect"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/danielgtaylor/huma/v2"
)

type registry struct{ huma.Registry }
type jobSchema struct{}

// MarshalJSON emits the schema definitions collected by the embedded registry.
func (r *registry) MarshalJSON() ([]byte, error) { return json.Marshal(r.Map()) }

// MarshalYAML emits the same definitions for Huma's YAML documentation endpoint.
func (r *registry) MarshalYAML() (any, error) { return r.Map(), nil }

// Schema describes the same variants serialized by scheduler.Job.MarshalJSON.
func (jobSchema) Schema(registry huma.Registry) *huma.Schema {
	deliverySchema := registry.Schema(reflect.TypeFor[scheduler.DeliveryJob](), false, "")
	variant := func(kind string) *huma.Schema {
		result := *deliverySchema
		result.Properties = maps.Clone(deliverySchema.Properties)
		kindSchema := *result.Properties["kind"]
		kindSchema.Enum = []any{kind}
		result.Properties["kind"] = &kindSchema
		return &result
	}
	return &huma.Schema{OneOf: []*huma.Schema{
		registry.Schema(reflect.TypeFor[scheduler.IndexJob](), true, ""),
		variant("notify"), variant("push"),
	}}
}

func newRegistry() *registry {
	inner := huma.NewMapRegistry("#/components/schemas/", func(value reflect.Type, hint string) string {
		for value.Kind() == reflect.Pointer {
			value = value.Elem()
		}
		name := huma.DefaultSchemaNamer(value, hint)
		if value.PkgPath() == "github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites" || strings.Contains(value.Name(), "/storage/favorites.") {
			return "Favorite" + name
		}
		return name
	})
	inner.RegisterTypeAlias(reflect.TypeFor[identity.Id](), reflect.TypeFor[string]())
	inner.RegisterTypeAlias(reflect.TypeFor[json.RawMessage](), reflect.TypeFor[map[string]any]())
	inner.RegisterTypeAlias(reflect.TypeFor[delivery.SecretUpdate](), reflect.TypeFor[*string]())
	inner.RegisterTypeAlias(reflect.TypeFor[scheduler.Job](), reflect.TypeFor[jobSchema]())
	result := &registry{Registry: inner}
	registerRuntimeSettingSchemas(result)
	return result
}

// Schema derives nullability from the same Go fields used to encode response bodies.
func (r *registry) Schema(value reflect.Type, allowRef bool, hint string) *huma.Schema {
	schema := r.Registry.Schema(value, allowRef, hint)
	for name, definition := range r.Map() {
		typeOf := r.TypeFromRef("#/components/schemas/" + name)
		if typeOf == nil || typeOf.Kind() != reflect.Struct {
			continue
		}
		if typeOf == reflect.TypeFor[settings.Info]() && len(definition.AllOf) == 0 {
			describeRuntimeSettingInfo(definition)
		}
		r.normalizeFields(typeOf, definition)
	}
	if value.Kind() == reflect.Slice {
		schema.Nullable = false
	}
	return schema
}

func (r *registry) normalizeFields(value reflect.Type, schema *huma.Schema) {
	for index := 0; index < value.NumField(); index++ {
		field := value.Field(index)
		if field.Anonymous {
			typeOf := field.Type
			if typeOf.Kind() == reflect.Pointer {
				typeOf = typeOf.Elem()
			}
			if typeOf.Kind() == reflect.Struct {
				r.normalizeFields(typeOf, schema)
			}
			continue
		}
		name, _, _ := strings.Cut(field.Tag.Get("json"), ",")
		property := schema.Properties[name]
		if property == nil {
			continue
		}
		if field.Type.Kind() == reflect.Pointer && property.Ref != "" && field.Tag.Get("nullable") != "false" {
			schema.Properties[name] = nullableSchema(property)
		}
		if field.Tag.Get("allowNull") == "true" && len(property.AnyOf) == 0 {
			schema.Properties[name] = nullableSchema(property)
		}
		if field.Type.Kind() == reflect.Slice {
			property.Nullable = false
		}
	}
}

func nullableSchema(schema *huma.Schema) *huma.Schema {
	return &huma.Schema{AnyOf: []*huma.Schema{schema, {Type: "null"}}}
}
