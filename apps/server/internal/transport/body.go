//go:build go1.27

package transport

import (
	"bytes"
	"encoding/json"
	"encoding/json/jsontext"
	"errors"
	"io"
	"net/http"
	"unicode/utf8"

	"golang.org/x/text/encoding/unicode"
)

var (
	// ErrReadFailed hides upstream response-reader diagnostics.
	ErrReadFailed = errors.New("response body could not be read")
	// ErrTooLarge identifies the decoded endpoint size boundary.
	ErrTooLarge = errors.New("response body exceeded the configured size limit")
	// ErrInvalidJSON identifies malformed bounded JSON without including its contents.
	ErrInvalidJSON = errors.New("response body was not valid JSON")
)

// BoundedBytes consumes and closes a transparently decoded HTTP response body.
// A read failure takes precedence over the observed size, except for an oversized length header.
func BoundedBytes(response *http.Response, maximum int64) ([]byte, error) {
	defer func(closeBody func() error) { _ = closeBody() }(response.Body.Close)
	if maximum < 0 || response.ContentLength > maximum {
		return nil, ErrTooLarge
	}
	limit := maximum
	if limit < int64(^uint64(0)>>1) {
		limit++
	}
	var result bytes.Buffer
	result.Grow(int(min(max(response.ContentLength, 0), maximum, 64*1024)))
	if _, err := result.ReadFrom(io.LimitReader(response.Body, limit)); err != nil {
		return nil, ErrReadFailed
	}
	if int64(result.Len()) > maximum {
		return nil, ErrTooLarge
	}
	return result.Bytes(), nil
}

// BoundedText decodes bounded bytes using compatible UTF-8 replacement groups.
func BoundedText(response *http.Response, maximum int64) (string, error) {
	body, err := BoundedBytes(response, maximum)
	if err != nil {
		return "", err
	}
	return LossyUtf8(body), nil
}

// LossyUtf8 replaces each invalid UTF-8 subsequence once, including truncated prefixes.
func LossyUtf8(body []byte) string {
	if utf8.Valid(body) {
		return string(body)
	}
	decoded, _ := unicode.UTF8.NewDecoder().Bytes(body)
	return string(decoded)
}

// ParseJSON retains numeric tokens while applying encoding/json validation.
func ParseJSON(body []byte) (any, error) {
	if !jsontext.Value(body).IsValid() {
		return nil, ErrInvalidJSON
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	var result any
	if err := decoder.Decode(&result); err != nil {
		return nil, ErrInvalidJSON
	}
	return result, nil
}

// BoundedJSON reads a bounded response before applying strict JSON validation.
func BoundedJSON(response *http.Response, maximum int64) (any, error) {
	body, err := BoundedBytes(response, maximum)
	if err != nil {
		return nil, err
	}
	return ParseJSON(body)
}
