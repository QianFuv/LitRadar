// Package openapi registers Go-owned API contracts with Huma and net/http.
package openapi

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"reflect"
	"strconv"

	"github.com/QianFuv/LitRadar"
	"github.com/danielgtaylor/huma/v2"
	"github.com/danielgtaylor/huma/v2/adapters/humago"
)

// Operation identifies a live handler and its stable public operation ID.
type Operation struct{ Method, Path, Id string }

type noBody struct{}
type replayBodyKey struct{}

type replayReader struct {
	io.Reader
	io.Closer
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
	return humago.New(mux, config)
}

// Generate registers the same typed operations as the live API without deployment resources.
func Generate(bindings []Operation) ([]byte, error) {
	api := New(http.NewServeMux())
	seen := make(map[string]bool, len(bindings))
	for _, binding := range bindings {
		key := binding.Method + " " + binding.Path
		if seen[key] {
			return nil, fmt.Errorf("duplicate API binding: %s", key)
		}
		seen[key] = true
		if err := Register(api, binding, nil); err != nil {
			return nil, err
		}
	}
	return json.Marshal(api.OpenAPI())
}

// registerOperation validates typed inputs and retains the handler's wire representation.
func registerOperation[Input, Body any](api huma.API, operation huma.Operation, handler http.HandlerFunc) {
	if reflect.TypeFor[Body]() != reflect.TypeFor[noBody]() {
		schema := api.OpenAPI().Components.Schemas.Schema(reflect.TypeFor[Body](), true, operation.OperationID+"Response")
		schema.Nullable = false
		if reflect.TypeFor[Body]().Kind() == reflect.Pointer {
			schema = nullableSchema(schema)
		}
		operation.Responses[strconv.Itoa(operation.DefaultStatus)] = &huma.Response{
			Description: http.StatusText(operation.DefaultStatus),
			Content: map[string]*huma.MediaType{
				"application/json": {Schema: schema},
			},
		}
	}
	if field, hasBody := reflect.TypeFor[Input]().FieldByName("Body"); hasBody {
		operation.Middlewares = append(operation.Middlewares, replayRequestBody)
		if field.Type.Kind() == reflect.Pointer {
			operation.RequestBody = &huma.RequestBody{Content: map[string]*huma.MediaType{
				"application/json": {Schema: nullableSchema(api.OpenAPI().Components.Schemas.Schema(field.Type, true, operation.OperationID+"Request"))},
			}}
		}
	}
	huma.Register[Input, huma.StreamResponse](api, operation, func(_ context.Context, _ *Input) (*huma.StreamResponse, error) {
		return &huma.StreamResponse{Body: func(ctx huma.Context) {
			request, writer := humago.Unwrap(ctx)
			if buffer, ok := ctx.Context().Value(replayBodyKey{}).(*bytes.Buffer); ok {
				request.Body = io.NopCloser(bytes.NewReader(buffer.Bytes()))
			}
			handler(writer, request)
		}}, nil
	})
	for _, code := range []string{"429", "503"} {
		if response := operation.Responses[code]; response != nil {
			response.Headers = map[string]*huma.Param{
				"Retry-After": {Description: "Seconds before retrying the request.", Schema: &huma.Schema{Type: "string"}},
			}
		}
	}
}

// nullableSchema permits an explicit JSON null without changing a shared object definition.
func nullableSchema(schema *huma.Schema) *huma.Schema {
	return &huma.Schema{AnyOf: []*huma.Schema{schema, {Type: "null"}}}
}

// replayRequestBody retains the bounded Huma input bytes for the business decoder.
func replayRequestBody(ctx huma.Context, next func(huma.Context)) {
	// ponytail: bodies are decoded twice; remove replay when handlers consume typed inputs.
	request, _ := humago.Unwrap(ctx)
	buffer := &bytes.Buffer{}
	original := request.Body
	if original != nil {
		request.Body = replayReader{Reader: io.TeeReader(original, buffer), Closer: original}
	}
	defer func() { request.Body = original }()
	next(huma.WithContext(ctx, context.WithValue(ctx.Context(), replayBodyKey{}, buffer)))
}
