package cfp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/platform/process"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

const obscuraProtocol = "litradar.cfp.page.v1"
const obscuraEval = "(() => { const html = document.documentElement ? document.documentElement.outerHTML : ''; if (html.length > 2097152) throw new Error('CFP capture too large'); return JSON.stringify({protocol:'litradar.cfp.page.v1',finalUrl:location.href,html}); })()"

type RefreshOptions struct {
	SourceTimeout, OverallTimeout time.Duration
	ObscuraPath, PdftotextPath    *string
}

func DefaultRefreshOptions() RefreshOptions {
	result := RefreshOptions{SourceTimeout: 90 * time.Second, OverallTimeout: 600 * time.Second}
	if value, has := os.LookupEnv("LITRADAR_OBSCURA_PATH"); has {
		result.ObscuraPath = &value
	}
	if value, has := os.LookupEnv("LITRADAR_PDFTOTEXT_PATH"); has {
		result.PdftotextPath = &value
	}
	return result
}

type LiveTransport struct {
	http           *HttpTransport
	options        RefreshOptions
	hasUsedObscura atomic.Bool
}

func NewLiveTransport(options RefreshOptions) *LiveTransport {
	return &LiveTransport{http: NewHttpTransport(), options: options}
}
func (transport *LiveTransport) Close() { transport.http.Close() }
func check(ctx context.Context, deadline time.Time) error {
	if ctx.Err() != nil {
		return ErrCancelled
	}
	if !time.Now().Before(deadline) {
		return ErrDeadline
	}
	return nil
}
func executable(path *string, fallback string) string {
	if path != nil {
		return *path
	}
	return fallback
}

func (transport *LiveTransport) httpDocument(ctx context.Context, config SourceConfig, url string, deadline time.Time) (Document, error) {
	response, err := transport.http.FetchBytes(ctx, config, url, deadline)
	if err != nil {
		return Document{}, err
	}
	if err = check(ctx, deadline); err != nil {
		return Document{}, err
	}
	if bytes.HasPrefix(response.Bytes, []byte("%PDF-")) {
		directory, err := os.MkdirTemp("", "litradar-cfp-")
		if err != nil {
			return Document{}, ErrHelper
		}
		defer os.RemoveAll(directory)
		input, output := filepath.Join(directory, "source.pdf"), filepath.Join(directory, "source.txt")
		if os.WriteFile(input, response.Bytes, 0600) != nil {
			return Document{}, ErrHelper
		}
		data, err := runHelper(ctx, process.Config{Path: executable(transport.options.PdftotextPath, "pdftotext"), Args: []string{"-enc", "UTF-8", "-eol", "unix", "-nopgbrk", input, output}}, deadline, output)
		if err != nil {
			return Document{}, err
		}
		if !utf8.Valid(data) {
			return Document{}, ErrEncoding
		}
		if strings.TrimSpace(string(data)) == "" {
			return Document{}, ErrUnrecognized
		}
		return Document{FinalUrl: response.FinalUrl, Text: string(data), Format: "pdf_text"}, nil
	}
	text, err := DecodeBody(response.Bytes, response.ContentType)
	if err != nil {
		return Document{}, err
	}
	return Document{FinalUrl: response.FinalUrl, Text: text, Format: "html"}, nil
}

func (transport *LiveTransport) obscuraDocument(ctx context.Context, config SourceConfig, url string, deadline time.Time) (Document, error) {
	if err := check(ctx, deadline); err != nil {
		return Document{}, err
	}
	if transport.hasUsedObscura.Swap(true) {
		return Document{}, ErrHelper
	}
	directory, err := os.MkdirTemp("", "litradar-cfp-")
	if err != nil {
		return Document{}, ErrHelper
	}
	defer os.RemoveAll(directory)
	output := filepath.Join(directory, "page.json")
	seconds := strconv.FormatInt(max(1, min(40, int64(time.Until(deadline)/time.Second))), 10)
	args := []string{"fetch", url, "--stealth", "--timeout", seconds, "--eval", obscuraEval, "--quiet", "--output", output}
	location, err := whatwg.NewParser().Parse(url)
	if err == nil && location.Hostname() == "link.springer.com" {
		args = append(args, "--wait-until", "domcontentloaded", "--wait", "0")
	}
	environment := []string{}
	for _, entry := range os.Environ() {
		name, _, _ := strings.Cut(entry, "=")
		if name == "OBSCURA_ALLOW_PRIVATE_NETWORK" || runtime.GOOS == "windows" && strings.EqualFold(name, "OBSCURA_ALLOW_PRIVATE_NETWORK") {
			continue
		}
		environment = append(environment, entry)
	}
	data, err := runHelper(ctx, process.Config{Path: executable(transport.options.ObscuraPath, "obscura"), Args: args, Environment: environment}, deadline, output)
	if err != nil {
		return Document{}, err
	}
	return decodeObscura(config, data)
}

func decodeObscura(config SourceConfig, data []byte) (Document, error) {
	if len(data) > MaxPageBytes {
		return Document{}, ErrTooLarge
	}
	if !jsonvalue.ValidJson(string(data)) {
		return Document{}, ErrHelper
	}
	var fields []string
	data = bytes.TrimSpace(data)
	if len(data) > 0 && data[0] == '[' {
		var values []*string
		if json.Unmarshal(data, &values) != nil || len(values) != 3 {
			return Document{}, ErrHelper
		}
		for _, value := range values {
			if value == nil {
				return Document{}, ErrHelper
			}
			fields = append(fields, *value)
		}
	} else {
		decoder := json.NewDecoder(bytes.NewReader(data))
		token, err := decoder.Token()
		if err != nil || token != json.Delim('{') {
			return Document{}, ErrHelper
		}
		values := map[string]string{}
		for decoder.More() {
			key, err := decoder.Token()
			if err != nil {
				return Document{}, ErrHelper
			}
			name := key.(string)
			if name != "protocol" && name != "finalUrl" && name != "html" {
				return Document{}, ErrHelper
			}
			if _, has := values[name]; has {
				return Document{}, ErrHelper
			}
			var raw json.RawMessage
			if decoder.Decode(&raw) != nil || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
				return Document{}, ErrHelper
			}
			var text string
			if json.Unmarshal(raw, &text) != nil {
				return Document{}, ErrHelper
			}
			values[name] = text
		}
		if len(values) != 3 {
			return Document{}, ErrHelper
		}
		fields = []string{values["protocol"], values["finalUrl"], values["html"]}
	}
	if fields[0] != obscuraProtocol || strings.TrimSpace(fields[2]) == "" {
		return Document{}, ErrUnrecognized
	}
	location, err := whatwg.NewParser().Parse(fields[1])
	if err != nil || !config.PermitsUrl(location) {
		return Document{}, ErrDisallowedUrl
	}
	return Document{FinalUrl: fields[1], Text: fields[2], Format: "html"}, nil
}

func canRender(err error, includesDeadline bool) bool {
	var failure SourceError
	if !errors.As(err, &failure) {
		return false
	}
	return failure == ErrRequest || failure == ErrChallenge || failure.Kind == "http_status" && (failure.Status == 403 || failure.Status == 429 || failure.Status == 503) || includesDeadline && (failure == ErrDeadline || failure == ErrUnrecognized)
}

// Fetch allows one rendered fallback across the discovery page and all its detail requests.
func (transport *LiveTransport) Fetch(ctx context.Context, config SourceConfig, url string, deadline time.Time) (Document, error) {
	if err := check(ctx, deadline); err != nil {
		return Document{}, err
	}
	document, err := transport.httpDocument(ctx, config, url, deadline)
	if err == nil && !(document.Format == "pdf_text" && url != config.DiscoveryUrl) {
		_, err = ParsePage(config, document, utcNow().Format("2006-01-02"), url == config.DiscoveryUrl)
	}
	if err == nil {
		return document, nil
	}
	originalError := err
	if !canRender(err, true) || transport.hasUsedObscura.Load() {
		return Document{}, err
	}
	document, err = transport.obscuraDocument(ctx, config, url, deadline)
	if err != nil {
		return Document{}, err
	}
	_, err = ParsePage(config, document, utcNow().Format("2006-01-02"), url == config.DiscoveryUrl)
	if errors.Is(err, ErrUnsupported) {
		return Document{}, originalError
	}
	return document, err
}

// runHelper retains tree ownership until cleanup, including after an apparently successful leader exit.
func runHelper(ctx context.Context, config process.Config, deadline time.Time, output string) ([]byte, error) {
	if err := check(ctx, deadline); err != nil {
		return nil, err
	}
	child, err := process.Start(context.Background(), config)
	if err != nil {
		return nil, ErrHelper
	}
	child.Stdin.Close()
	defer child.Close()
	for {
		failure := check(ctx, deadline)
		if failure == nil {
			if metadata, err := os.Stat(output); err == nil && metadata.Size() > MaxPageBytes {
				failure = ErrTooLarge
			}
		}
		if failure != nil {
			if child.Close() != nil {
				return nil, ErrHelper
			}
			return nil, failure
		}
		hasExited, exitError := child.Poll()
		if hasExited {
			if child.Close() != nil || exitError != nil {
				return nil, ErrHelper
			}
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	file, err := os.Open(output)
	if err != nil {
		return nil, ErrHelper
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, MaxPageBytes+1))
	if err != nil {
		return nil, ErrHelper
	}
	if len(data) > MaxPageBytes {
		return nil, ErrTooLarge
	}
	if !time.Now().Before(deadline) {
		return nil, ErrDeadline
	}
	return data, nil
}
func utcNow() time.Time {
	now := time.Now().UTC()
	if now.Before(time.Unix(0, 0)) {
		return time.Unix(0, 0).UTC()
	}
	return now
}
