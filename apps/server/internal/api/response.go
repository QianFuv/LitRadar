// Package api preserves LitRadar's public HTTP operations and response contracts.
package api

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"strconv"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/admission"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/danielgtaylor/huma/v2"
	"modernc.org/sqlite"
)

type apiError struct {
	status     int
	detail     string
	details    []error
	retryAfter *uint64
}

const statusClientClosedRequest = 499

func (failure *apiError) Error() string { return failure.detail }

func (failure *apiError) problem() error {
	model := failure.model()
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
	model := failure.model()
	if err := httpwire.JSON(writer, failure.status, model); err != nil {
		slog.Error("http.response.failed", "error_kind", "problem_write_failed")
	}
}

// model builds Huma's standard error directly from typed details.
func (failure *apiError) model() huma.StatusError {
	detail := failure.detail
	if detail == "" {
		detail = http.StatusText(failure.status)
	}
	return huma.NewError(failure.status, detail, failure.details...)
}

func mapExecutorError(err error) *apiError {
	if errors.Is(err, context.Canceled) {
		return &apiError{status: statusClientClosedRequest, detail: "Client closed request"}
	}
	if errors.Is(err, admission.ErrClosed) || errors.Is(err, context.DeadlineExceeded) {
		return serviceUnavailable()
	}
	return internalError()
}

// requestFailure preserves cancellation and records safe diagnostics before public error mapping.
func requestFailure(ctx context.Context, err error, classify func(error) *apiError) *apiError {
	if errors.Is(ctx.Err(), context.Canceled) || errors.Is(err, context.Canceled) {
		return mapExecutorError(context.Canceled)
	}
	failure := classify(err)
	if failure.status >= 500 {
		kind := "internal_error"
		requestID, _ := ctx.Value(requestIDKey{}).(string)
		attributes := []slog.Attr{slog.String("event", "http.operation.failed"), slog.String("component", "http"), slog.String("request_id", requestID), slog.Int("status", failure.status)}
		errorType := fmt.Sprintf("%T", err)
		switch {
		case errors.Is(err, context.DeadlineExceeded):
			kind = "deadline_exceeded"
		case errors.Is(err, admission.ErrClosed):
			kind = "admission_closed"
		default:
			if sqliteError, ok := errors.AsType[*sqlite.Error](err); ok {
				kind = "sqlite_error"
				attributes = append(attributes, slog.Int("sqlite_code", sqliteError.Code()))
				errorType = "*sqlite.Error"
			}
		}
		attributes = append(attributes, slog.String("error_kind", kind), slog.String("error_type", errorType))
		slog.LogAttrs(ctx, slog.LevelError, "http.operation.failed", attributes...)
	}
	return failure
}

func mapIndexError(err error) *apiError {
	var missing query.NotFound
	var invalid query.InvalidInput
	var sort query.UnsupportedSortField
	switch {
	case errors.Is(err, config.ErrNotFound), errors.Is(err, config.ErrNoDatabases), errors.As(err, &missing):
		return &apiError{status: 404, detail: publicErrorDetail(err)}
	case errors.Is(err, config.ErrMultipleDatabases), errors.Is(err, config.ErrInvalidName), errors.As(err, &invalid), errors.As(err, &sort), errors.Is(err, query.ErrUnsupportedArticleSort), errors.Is(err, query.ErrInvalidCursor), errors.Is(err, query.ErrInvalidSearchExpression):
		return badRequest(publicErrorDetail(err))
	case errors.Is(err, query.ErrLegacyWeeklyLimit):
		return &apiError{status: 413, detail: publicErrorDetail(err)}
	default:
		return internalError()
	}
}
