// Package openapi configures Go-owned API contracts for Huma and net/http.
package openapi

import (
	"encoding/json"
	"github.com/QianFuv/LitRadar"
	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/danielgtaylor/huma/v2"
	"github.com/danielgtaylor/huma/v2/adapters/humago"
	"io"
	"net/http"
)

// Operation identifies a live handler and its stable public operation ID.
type Operation struct {
	// Method identifies the HTTP request method.
	Method string
	// Path identifies the ServeMux route template.
	Path string
	// ID identifies the generated OpenAPI operation.
	ID string
}

// New configures the public documentation and disables response schema injection.
func New(mux *http.ServeMux) huma.API {
	config := huma.DefaultConfig("LitRadar API", litradar.Version())
	config.CreateHooks = nil
	config.SchemasPath = ""
	config.Info.Description = "LitRadar HTTP API, generated from Go request and response types."
	config.Info.License = &huma.License{Name: "MIT", Identifier: "MIT"}
	config.Components.SecuritySchemes = map[string]*huma.SecurityScheme{
		"bearer_auth":    {Type: "http", Scheme: "bearer"},
		"session_cookie": {Type: "apiKey", In: "cookie", Name: "litradar_session"},
	}
	config.Components.Schemas = newRegistry()
	config.Formats = map[string]huma.Format{"application/json": {Marshal: func(writer io.Writer, value any) error {
		encoded, err := jsonvalue.EncodeJSON(value)
		if err != nil {
			return err
		}
		_, err = io.WriteString(writer, encoded)
		return err
	}, Unmarshal: json.Unmarshal}}
	config.Formats["json"] = config.Formats["application/json"]
	config.Formats["application/problem+json"] = config.Formats["application/json"]
	return humago.New(mux, config)
}
