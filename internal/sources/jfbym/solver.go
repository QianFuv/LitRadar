// Package jfbym implements the domestic CNKI dual-image slider solver boundary.
package jfbym

import (
	"bytes"
	"context"
	"crypto/aes"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const (
	// ApiUrl is the verified upstream custom recognition endpoint.
	ApiUrl = "https://api.jfbym.com/api/YmServer/customApi"
	// DualSliderType identifies the CNKI blockPuzzle solver.
	DualSliderType = "20111"
	// SuccessCode is the exact integer recognition-success discriminator.
	SuccessCode     int64 = 10000
	maximumPoint          = 10000
	maximumResponse       = 256 * 1024
)

// ErrorKind distinguishes configuration, transport and unusable upstream payloads.
type ErrorKind string

const (
	Configuration   ErrorKind = "configuration"
	Request         ErrorKind = "request"
	InvalidResponse ErrorKind = "invalid_response"
)

// Error contains a fixed diagnostic without tokens, puzzle secrets or images.
type Error struct {
	Kind    ErrorKind
	Message string
}

func (err *Error) Error() string { return err.Message }

func failure(kind ErrorKind, message string) error { return &Error{kind, message} }

// Solver resolves a gap position within the calling operation's cancellation boundary.
type Solver interface {
	SolveDualImage(context.Context, string, string) (float64, error)
}

// Fixture deterministically fails a configured number of nonempty challenges before succeeding.
type Fixture struct {
	mutex             sync.Mutex
	distance          float64
	remainingFailures uint64
}

// NewFixture creates a solver with a fixed distance and initial failure budget.
func NewFixture(distance float64, failures uint64) *Fixture {
	return &Fixture{distance: distance, remainingFailures: failures}
}

// SolveDualImage preserves input-validation precedence over the fixture failure budget.
func (solver *Fixture) SolveDualImage(ctx context.Context, slideImage, backgroundImage string) (float64, error) {
	if strings.TrimSpace(slideImage) == "" || strings.TrimSpace(backgroundImage) == "" {
		return 0, failure(InvalidResponse, "jfbym fixture requires non-empty dual images")
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	solver.mutex.Lock()
	defer solver.mutex.Unlock()
	if solver.remainingFailures > 0 {
		solver.remainingFailures--
		return 0, failure(Request, "jfbym fixture forced failure")
	}
	return solver.distance, nil
}

// Live uses a private client with an explicit proxy decision and no redirects.
type Live struct {
	token            string
	client           *http.Client
	requestTimeout   time.Duration
	deadline         time.Time
	apiUrl, typeCode string
}

// NewLive preserves a minimum one-second HTTP timeout and a shared optional article deadline.
func NewLive(token string, timeoutSeconds uint64, proxy transport.Proxy, deadline time.Time) (*Live, error) {
	if strings.TrimSpace(token) == "" {
		return nil, failure(Configuration, "jfbym token is required")
	}
	wire, err := proxy.ClientTransport()
	if err != nil {
		return nil, failure(Request, err.Error())
	}
	return &Live{token: token, client: &http.Client{Transport: wire, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, requestTimeout: (transport.Delay{Seconds: max(timeoutSeconds, 1)}).Duration(), deadline: deadline, apiUrl: ApiUrl, typeCode: DualSliderType}, nil
}

// Close releases idle sockets owned by this solver.
func (solver *Live) Close() { solver.client.CloseIdleConnections() }

func (solver Live) String() string       { return "LiveJfbymSolver(token=[REDACTED])" }
func (solver Live) GoString() string     { return solver.String() }
func (solver Live) LogValue() slog.Value { return slog.StringValue(solver.String()) }

// SolveDualImage posts one bounded recognition request, preserving status-before-body validation.
func (solver *Live) SolveDualImage(ctx context.Context, slideImage, backgroundImage string) (float64, error) {
	if err := transport.EnsureArticleDeadline(solver.deadline); err != nil {
		return 0, failure(Request, err.Error())
	}
	if err := ctx.Err(); err != nil {
		return 0, failure(Request, "article access deadline expired")
	}
	slideImage, backgroundImage, err := prepareDualImages(slideImage, backgroundImage)
	if err != nil {
		return 0, err
	}
	payload, err := json.Marshal(map[string]string{"token": solver.token, "type": solver.typeCode, "slide_image": slideImage, "background_image": backgroundImage})
	if err != nil {
		return 0, failure(Request, "jfbym request could not be encoded")
	}
	timeout, err := transport.RequestTimeout(solver.requestTimeout, solver.deadline)
	if err != nil {
		return 0, failure(Request, err.Error())
	}
	requestContext, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	request, err := http.NewRequestWithContext(requestContext, http.MethodPost, solver.apiUrl, bytes.NewReader(payload))
	if err != nil {
		return 0, failure(Request, "jfbym request could not be constructed")
	}
	request.Header.Set("Content-Type", "application/json")
	response, err := solver.client.Do(request)
	if err != nil {
		return 0, failure(Request, "jfbym request failed")
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return 0, failure(Request, fmt.Sprintf("jfbym HTTP status %d", response.StatusCode))
	}
	return recognitionDistance(response)
}

// StripDataUrlBase64 removes an optional case-insensitive data-URL prefix.
func StripDataUrlBase64(value string) string {
	value = strings.TrimSpace(value)
	if len(value) >= 5 && strings.EqualFold(value[:5], "data:") {
		if _, payload, hasComma := strings.Cut(value, ","); hasComma {
			return strings.TrimSpace(payload)
		}
	}
	return value
}

// EncryptPointJson preserves AES-128 ECB/PKCS7 and the exact compact x-then-y JSON payload.
func EncryptPointJson(secretKey string, x, y int32) (string, error) {
	if len(secretKey) != 16 {
		return "", failure(InvalidResponse, fmt.Sprintf("captcha secretKey must be 16 bytes, got %d", len(secretKey)))
	}
	cipher, err := aes.NewCipher([]byte(secretKey))
	if err != nil {
		return "", failure(InvalidResponse, "captcha secretKey is invalid")
	}
	plaintext := []byte(fmt.Sprintf(`{"x":%d,"y":%d}`, x, y))
	padding := aes.BlockSize - len(plaintext)%aes.BlockSize
	plaintext = append(plaintext, bytes.Repeat([]byte{byte(padding)}, padding)...)
	for offset := 0; offset < len(plaintext); offset += aes.BlockSize {
		cipher.Encrypt(plaintext[offset:offset+aes.BlockSize], plaintext[offset:offset+aes.BlockSize])
	}
	return base64.StdEncoding.EncodeToString(plaintext), nil
}

func objectField(value any, key string) (any, bool) {
	object, isObject := value.(map[string]any)
	if !isObject {
		return nil, false
	}
	result, exists := object[key]
	return result, exists
}

func responseCode(payload any) (int64, bool) {
	value, _ := objectField(payload, "code")
	number, isNumber := value.(json.Number)
	if !isNumber {
		return 0, false
	}
	parsed, err := domain.ParseNumber(number)
	if err != nil {
		return 0, false
	}
	if signed, hasSigned := parsed.AsInt64(); hasSigned {
		return signed, true
	}
	unsigned, hasUnsigned := parsed.AsUint64()
	return int64(unsigned), hasUnsigned
}

var decimalFloat = regexp.MustCompile(`^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$`)

func sliderNumber(value any) (float64, bool) {
	var text string
	switch value := value.(type) {
	case json.Number:
		parsed, err := domain.ParseNumber(value)
		return parsed.AsFloat64(), err == nil
	case string:
		text = strings.TrimSpace(value)
	default:
		return 0, false
	}
	switch strings.ToLower(text) {
	case "nan", "+nan", "-nan":
		return math.NaN(), true
	case "inf", "+inf", "infinity", "+infinity":
		return math.Inf(1), true
	case "-inf", "-infinity":
		return math.Inf(-1), true
	}
	if !decimalFloat.MatchString(text) {
		return 0, false
	}
	parsed, err := strconv.ParseFloat(text, 64)
	return parsed, err == nil || errors.Is(err, strconv.ErrRange)
}

// ParseSliderDistance accepts only successful responses and their nested numeric gap field.
func ParseSliderDistance(payload any) (float64, error) {
	if code, hasCode := responseCode(payload); !hasCode || code != SuccessCode {
		return 0, failure(InvalidResponse, "jfbym response did not report recognition success")
	}
	data, _ := objectField(payload, "data")
	value, exists := objectField(data, "data")
	if !exists {
		return 0, failure(InvalidResponse, "jfbym response missing slider distance")
	}
	distance, isValid := sliderNumber(value)
	if !isValid {
		return 0, failure(InvalidResponse, "jfbym response has invalid slider distance")
	}
	return validateDistance(distance)
}

func validateDistance(value float64) (float64, error) {
	if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || value > maximumPoint {
		return 0, failure(InvalidResponse, "jfbym response has out-of-range slider distance")
	}
	return value, nil
}

// PointXCandidates returns rounded x, then +1, -1, +2, -2 within the accepted range.
func PointXCandidates(rawDistance float64) ([]int32, error) {
	distance, err := validateDistance(rawDistance)
	if err != nil {
		return nil, err
	}
	rounded := int32(math.Round(distance))
	result := make([]int32, 0, 5)
	for _, offset := range []int32{0, 1, -1, 2, -2} {
		value := rounded + offset
		if value >= 0 && value <= maximumPoint {
			result = append(result, value)
		}
	}
	return result, nil
}

// recognitionDistance preserves bounded JSON error classes and exact recognition code diagnostics.
func recognitionDistance(response *http.Response) (float64, error) {
	body, err := transport.BoundedJson(response, maximumResponse)
	if err != nil {
		switch err {
		case transport.ErrTooLarge:
			return 0, failure(InvalidResponse, "jfbym response exceeded the configured size limit")
		case transport.ErrReadFailed:
			return 0, failure(Request, "jfbym response body could not be read")
		default:
			return 0, failure(InvalidResponse, "jfbym response is not valid JSON")
		}
	}
	code, hasCode := responseCode(body)
	if !hasCode || code != SuccessCode {
		formatted := "None"
		if hasCode {
			formatted = fmt.Sprintf("Some(%d)", code)
		}
		return 0, failure(InvalidResponse, "jfbym recognition failed with code "+formatted)
	}
	return ParseSliderDistance(body)
}

// prepareDualImages strips data prefixes before requiring both recognition images.
func prepareDualImages(slideImage, backgroundImage string) (string, string, error) {
	slideImage = StripDataUrlBase64(slideImage)
	backgroundImage = StripDataUrlBase64(backgroundImage)
	if slideImage == "" || backgroundImage == "" {
		return "", "", failure(InvalidResponse, "jfbym dual-image solve requires slide and background images")
	}
	return slideImage, backgroundImage, nil
}
