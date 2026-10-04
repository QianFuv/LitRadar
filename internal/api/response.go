// Package api preserves LitRadar's public HTTP operations and response contracts.
package api

import (
	"context"
	"errors"
	"net/http"
	"strconv"

	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

type apiError struct {
	status     int
	detail     string
	structured any
	retryAfter *uint64
	isPlain    bool
}

func (failure *apiError) Error() string  { return failure.detail }
func badRequest(detail string) *apiError { return &apiError{status: 400, detail: detail} }
func internalError() *apiError           { return &apiError{status: 500, detail: "Internal Server Error"} }
func serviceUnavailable() *apiError {
	retry := uint64(5)
	return &apiError{status: 503, detail: "Service temporarily unavailable", retryAfter: &retry}
}

func (failure *apiError) write(writer http.ResponseWriter) {
	if failure.isPlain {
		writer.Header().Set("Content-Type", "text/plain; charset=utf-8")
		writer.WriteHeader(failure.status)
		_, _ = writer.Write([]byte(failure.detail))
		return
	}
	if failure.structured != nil {
		_ = httpwire.JSON(writer, failure.status, map[string]any{"detail": failure.structured})
		return
	}
	if failure.retryAfter != nil {
		writer.Header().Set("Retry-After", strconv.FormatUint(*failure.retryAfter, 10))
	}
	_ = httpwire.JSON(writer, failure.status, httpwire.ErrorEnvelope{
		Detail: failure.detail, Code: errorCode(failure.status), Retryable: failure.retryAfter != nil,
	})
}

func errorCode(status int) string {
	switch status {
	case 400:
		return "bad_request"
	case 401:
		return "unauthorized"
	case 403:
		return "forbidden"
	case 404:
		return "not_found"
	case 409:
		return "conflict"
	case 413:
		return "payload_too_large"
	case 429:
		return "rate_limited"
	case 502:
		return "bad_gateway"
	case 503:
		return "service_unavailable"
	case 500:
		return "internal_server_error"
	default:
		return "request_failed"
	}
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
