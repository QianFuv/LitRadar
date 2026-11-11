// Package httpwire implements the shared HTTP response conventions of the API.
package httpwire

import (
	"io"
	"net/http"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/danielgtaylor/huma/v2"
)

// JSON encodes before sending headers so serialization failure cannot emit partial success.
func JSON(writer http.ResponseWriter, status int, value any) error {
	encoded, err := jsonvalue.EncodeJson(value)
	if err != nil {
		return err
	}
	contentType := "application/json"
	if filtered, ok := value.(huma.ContentTypeFilter); ok {
		contentType = filtered.ContentType(contentType)
	}
	writer.Header().Set("Content-Type", contentType)
	writer.WriteHeader(status)
	_, err = io.WriteString(writer, encoded)
	return err
}

// Redirect emits the empty, non-cacheable temporary redirect used by article access.
func Redirect(writer http.ResponseWriter, location string) {
	writer.Header().Set("Location", location)
	writer.Header().Set("Cache-Control", "private, no-store")
	writer.WriteHeader(http.StatusTemporaryRedirect)
}

// File streams an already resolved asset and delegates range and HEAD semantics to net/http.
// The caller owns path validation, representation selection and the reader's lifetime.
func File(writer http.ResponseWriter, request *http.Request, name, mediaType, cacheControl string, modified time.Time, content io.ReadSeeker) {
	writer.Header().Set("Content-Type", mediaType)
	writer.Header().Set("Cache-Control", cacheControl)
	http.ServeContent(writer, request, name, modified, content)
}
