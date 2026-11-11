// Package api preserves LitRadar's public HTTP operations and response contracts.
package api

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"slices"
	"strconv"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/admission"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/danielgtaylor/huma/v2"
)

type apiError struct {
	status     int
	detail     string
	structured any
	retryAfter *uint64
}

func (failure *apiError) Error() string { return failure.detail }

func (failure *apiError) problem() error {
	detail, details := failure.problemDetails()
	model := huma.NewError(failure.status, detail, details...)
	if failure.retryAfter != nil {
		return huma.ErrorWithHeaders(model, http.Header{"Retry-After": {strconv.FormatUint(*failure.retryAfter, 10)}})
	}
	if failure.status == 503 || failure.status == 429 {
		return huma.ErrorWithHeaders(model, http.Header{"Retry-After": {"5"}})
	}
	return model
}
func badRequest(detail string) *apiError { return &apiError{status: 400, detail: detail} }
func internalError() *apiError           { return &apiError{status: 500, detail: "Internal Server Error"} }
func serviceUnavailable() *apiError {
	retry := uint64(5)
	return &apiError{status: 503, detail: "Service temporarily unavailable", retryAfter: &retry}
}

// write emits Huma's default problem model for business and extraction failures.
func (failure *apiError) write(writer http.ResponseWriter) {
	if failure.retryAfter != nil {
		writer.Header().Set("Retry-After", strconv.FormatUint(*failure.retryAfter, 10))
	} else if failure.status == http.StatusServiceUnavailable || failure.status == http.StatusTooManyRequests {
		writer.Header().Set("Retry-After", "5")
	}
	detail, details := failure.problemDetails()
	model := huma.NewError(failure.status, detail, details...)
	if err := httpwire.JSON(writer, failure.status, model); err != nil {
		slog.Error("http.response.failed", "error_kind", "problem_write_failed")
	}
}

// problemDetails retains classified business diagnostics within the standard errors array.
func (failure *apiError) problemDetails() (string, []error) {
	fields := map[string]string{}
	switch structured := failure.structured.(type) {
	case map[string]string:
		fields = structured
	case map[string]any:
		for name, value := range structured {
			if text, ok := value.(string); ok {
				fields[name] = text
			}
		}
	}
	detail := failure.detail
	if message := fields["message"]; message != "" {
		detail = message
	}
	if detail == "" {
		detail = http.StatusText(failure.status)
	}
	names := make([]string, 0, len(fields))
	for name := range fields {
		if name != "message" {
			names = append(names, name)
		}
	}
	slices.Sort(names)
	details := make([]error, 0, len(names))
	for _, name := range names {
		details = append(details, &huma.ErrorDetail{Location: name, Message: fields[name]})
	}
	return detail, details
}

func mapExecutorError(err error) *apiError {
	if errors.Is(err, admission.ErrClosed) || errors.Is(err, context.DeadlineExceeded) {
		return serviceUnavailable()
	}
	return internalError()
}

func mapIndexError(err error) *apiError {
	var missing query.NotFound
	var invalid query.InvalidInput
	var sort query.UnsupportedSortField
	switch {
	case errors.Is(err, config.ErrNotFound), errors.Is(err, config.ErrNoDatabases), errors.As(err, &missing):
		return &apiError{status: 404, detail: err.Error()}
	case errors.Is(err, config.ErrMultipleDatabases), errors.Is(err, config.ErrInvalidName), errors.As(err, &invalid), errors.As(err, &sort), errors.Is(err, query.ErrUnsupportedArticleSort), errors.Is(err, query.ErrInvalidCursor), errors.Is(err, query.ErrInvalidSearchExpression):
		return badRequest(err.Error())
	case errors.Is(err, query.ErrLegacyWeeklyLimit):
		return &apiError{status: 413, detail: err.Error()}
	default:
		return internalError()
	}
}
