package openapi

import (
	"reflect"

	"github.com/danielgtaylor/huma/v2"
)

type scheduledIndexSpec struct {
	Kind string `json:"kind" enum:"index"`
	ScheduledIndexJob
}

type scheduledNotifySpec struct {
	Kind string `json:"kind" enum:"notify"`
	ScheduledDeliveryJob
}

type scheduledPushSpec struct {
	Kind string `json:"kind" enum:"push"`
	ScheduledDeliveryJob
}

// Schema exposes the three scheduler command variants to request validation and OpenAPI.
func (ScheduledJobSpec) Schema(registry huma.Registry) *huma.Schema {
	return &huma.Schema{OneOf: []*huma.Schema{
		registry.Schema(reflect.TypeFor[scheduledIndexSpec](), true, ""),
		registry.Schema(reflect.TypeFor[scheduledNotifySpec](), true, ""),
		registry.Schema(reflect.TypeFor[scheduledPushSpec](), true, ""),
	}}
}

// TransformSchema permits null jobs on migrated legacy scheduler rows.
func (ScheduledTaskInfo) TransformSchema(_ huma.Registry, schema *huma.Schema) *huma.Schema {
	schema.Properties["job"] = nullableSchema(schema.Properties["job"])
	return schema
}

// TransformSchema permits a nullable replacement job in partial scheduler updates.
func (ScheduledTaskUpdate) TransformSchema(_ huma.Registry, schema *huma.Schema) *huma.Schema {
	schema.Properties["job"] = nullableSchema(schema.Properties["job"])
	return schema
}
