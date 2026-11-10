package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"net/http"
	"reflect"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/danielgtaylor/huma/v2"
	"github.com/danielgtaylor/huma/v2/adapters/humago"
)

// RequestMetadata retains request context for credential verification and audit metadata.
type RequestMetadata struct {
	// Authorization carries the Authorization header.
	Authorization string `header:"Authorization" hidden:"true"`
	// Cookie carries the Cookie header.
	Cookie  string `header:"Cookie" hidden:"true"`
	request *http.Request
}

func cleanText(value *string) *string {
	if value == nil {
		return nil
	}
	text := strings.TrimSpace(*value)
	if text == "" {
		return nil
	}
	return &text
}

func textOr(value *string, fallback string) string {
	if value != nil {
		return *value
	}
	return fallback
}

func numberOr(value *int64, fallback int64) int64 {
	if value != nil {
		return *value
	}
	return fallback
}

// Resolve binds transport metadata after Huma has decoded the operation input.
func (metadata *RequestMetadata) Resolve(ctx huma.Context) []error {
	metadata.request, _ = humago.Unwrap(ctx)
	return nil
}

type output[Body any] struct {
	// Body contains the decoded operation payload.
	Body Body
}

type nullableBody[Body any] struct{ value *Body }

// MarshalJSON emits the value or an explicit JSON null for absent resources.
func (body nullableBody[Body]) MarshalJSON() ([]byte, error) {
	encoded, err := jsonvalue.EncodeJson(body.value)
	return []byte(encoded), err
}

// Schema describes the same optional resource carried by the response body.
func (nullableBody[Body]) Schema(registry huma.Registry) *huma.Schema {
	return &huma.Schema{AnyOf: []*huma.Schema{registry.Schema(reflect.TypeFor[Body](), true, ""), {Type: "null"}}}
}

type optional[Value any] struct {
	value Value
	isSet bool
}

// Schema exposes the wrapped parameter's scalar contract to Huma.
func (optional[Value]) Schema(registry huma.Registry) *huma.Schema {
	return registry.Schema(reflect.TypeFor[Value](), false, "")
}

// Receiver exposes the scalar decoded by Huma without a second query parser.
func (value *optional[Value]) Receiver() reflect.Value { return reflect.ValueOf(&value.value).Elem() }

// OnParamSet records whether Huma found the parameter in the request.
func (value *optional[Value]) OnParamSet(isSet bool, _ any) { value.isSet = isSet }

// Pointer preserves omitted parameters when constructing business queries.
func (value optional[Value]) Pointer() *Value {
	if !value.isSet {
		return nil
	}
	return &value.value
}

func typedRoute[Input, Output any](binding openapi.Operation, handler func(context.Context, *Input) (*Output, error)) route {
	return route{operation: binding, register: func(api huma.API) error {
		operation, err := openapi.Describe(binding)
		if err != nil {
			return err
		}
		if field, ok := reflect.TypeFor[Output]().FieldByName("Body"); ok {
			for _, response := range operation.Responses {
				for media, content := range response.Content {
					if media != "application/json" && content.Schema == nil {
						content.Schema = huma.SchemaFromField(api.OpenAPI().Components.Schemas, field, binding.ID+"Response")
					}
				}
			}
		}
		huma.Register(api, operation, handler)
		if binding.ID == "oidc_login" || binding.ID == "oidc_callback" || binding.ID == "oidc_native_login" || binding.ID == "redirect_article_abstract" {
			operation.Responses[strconv.Itoa(operation.DefaultStatus)].Content = nil
		}
		for _, code := range []string{"429", "503"} {
			if response := operation.Responses[code]; response != nil {
				response.Headers = map[string]*huma.Param{"Retry-After": {Description: "Seconds before retrying the request.", Schema: &huma.Schema{Type: "string"}}}
			}
		}
		return nil
	}}
}

func admitted[Body any](ctx context.Context, pool *executor.Pool, work func() (Body, error), classify func(error) *apiError) (*output[Body], error) {
	type outcome struct {
		value Body
		err   error
	}
	result, err := executor.Run(ctx, pool, func() (outcome, error) {
		value, failure := work()
		return outcome{value, failure}, nil
	})
	if err != nil {
		return nil, mapExecutorError(err).problem()
	}
	if result.err != nil {
		return nil, classify(result.err).problem()
	}
	return &output[Body]{Body: result.value}, nil
}
