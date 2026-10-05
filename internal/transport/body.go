package transport

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"golang.org/x/text/encoding/unicode"
)

var (
	// ErrReadFailed hides upstream response-reader diagnostics.
	ErrReadFailed = errors.New("response body could not be read")
	// ErrTooLarge identifies the decoded endpoint size boundary.
	ErrTooLarge = errors.New("response body exceeded the configured size limit")
	// ErrInvalidJson identifies malformed bounded JSON without including its contents.
	ErrInvalidJson = errors.New("response body was not valid JSON")
)

// BoundedBytes consumes and closes a transparently decoded HTTP response body.
// A read failure takes precedence over the observed size, except for an oversized length header.
func BoundedBytes(response *http.Response, maximum int64) ([]byte, error) {
	defer response.Body.Close()
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

// BoundedText decodes bounded bytes using Rust-compatible UTF-8 replacement groups.
func BoundedText(response *http.Response, maximum int64) (string, error) {
	body, err := BoundedBytes(response, maximum)
	if err != nil {
		return "", err
	}
	return LossyUtf8(body), nil
}

// LossyUtf8 replaces each invalid UTF-8 subsequence once, including truncated prefixes.
func LossyUtf8(body []byte) string {
	decoded, _ := unicode.UTF8.NewDecoder().Bytes(body)
	return string(decoded)
}

// ParseJson retains exact integer tokens and rejects serde-incompatible lossy input.
func ParseJson(body []byte) (any, error) {
	masked, isValid := maskJsonNumbers(body)
	if !isValid || !jsonvalue.ValidJson(string(masked)) {
		return nil, ErrInvalidJson
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	var result any
	if err := decoder.Decode(&result); err != nil {
		return nil, ErrInvalidJson
	}
	return result, nil
}

func maskJsonNumbers(body []byte) ([]byte, bool) {
	masked := make([]byte, 0, len(body))
	for index := 0; index < len(body); {
		start := index
		if body[index] == '"' {
			index++
			for index < len(body) {
				character := body[index]
				index++
				if character == '\\' && index < len(body) {
					index++
				} else if character == '"' {
					break
				}
			}
			masked = append(masked, body[start:index]...)
			continue
		}
		if body[index] == '-' || body[index] >= '0' && body[index] <= '9' {
			index++
			for index < len(body) && (body[index] >= '0' && body[index] <= '9' || body[index] == '-' || body[index] == '+' || body[index] == '.' || body[index] == 'e' || body[index] == 'E') {
				index++
			}
			if _, err := domain.ParseNumber(json.Number(body[start:index])); err != nil {
				return nil, false
			}
			masked = append(masked, '0')
			continue
		}
		masked = append(masked, body[index])
		index++
	}
	return masked, true
}

// BoundedJson reads a bounded response before applying strict JSON validation.
func BoundedJson(response *http.Response, maximum int64) (any, error) {
	body, err := BoundedBytes(response, maximum)
	if err != nil {
		return nil, err
	}
	return ParseJson(body)
}
