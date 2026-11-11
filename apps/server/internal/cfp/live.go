package cfp

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/process"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
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
	http           *HTTPTransport
	options        RefreshOptions
	hasUsedObscura atomic.Bool
}

func NewLiveTransport(options RefreshOptions) *LiveTransport {
	return &LiveTransport{http: NewHTTPTransport(), options: options}
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
		defer func() {
			if err := removeCaptureDirectory(directory); err != nil {
				slog.Warn("cleanup.failed", "operation", "os.RemoveAll", "error", err)
			}
		}()
		input, output := filepath.Join(directory, "source.pdf"), filepath.Join(directory, "source.txt")
		root, err := os.OpenRoot(directory)
		if err != nil {
			return Document{}, ErrHelper
		}
		defer func() { _ = root.Close() }()
		if root.WriteFile("source.pdf", response.Bytes, 0600) != nil {
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
		return Document{FinalURL: response.FinalURL, Text: string(data), Format: "pdf_text"}, nil
	}
	text, err := DecodeBody(response.Bytes, response.ContentType)
	if err != nil {
		return Document{}, err
	}
	return Document{FinalURL: response.FinalURL, Text: text, Format: "html"}, nil
}

// obscuraDocument consumes one browser attempt after cancellation and deadline admission.
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
	defer func() {
		if err := removeCaptureDirectory(directory); err != nil {
			slog.Warn("cleanup.failed", "operation", "os.RemoveAll", "error", err)
		}
	}()
	output := filepath.Join(directory, "page.json")
	seconds := strconv.FormatInt(max(1, min(40, int64(time.Until(deadline)/time.Second))), 10)
	args := []string{"fetch", url, "--stealth", "--timeout", seconds, "--eval", obscuraEval, "--quiet", "--output", output}
	location, err := whatwg.NewParser().Parse(url)
	if err == nil && location.Hostname() == "link.springer.com" {
		args = append(args, "--wait-until", "domcontentloaded", "--wait", "0")
	}
	environment := obscuraEnvironment()
	data, err := runHelper(ctx, process.Config{Path: executable(transport.options.ObscuraPath, "obscura"), Args: args, Environment: environment}, deadline, output)
	if err != nil {
		return Document{}, err
	}
	return decodeObscura(config, data)
}

// decodeObscura preserves byte/schema/protocol/final-URL error priority and original URL spelling.
func decodeObscura(config SourceConfig, data []byte) (Document, error) {
	if len(data) > MaxPageBytes {
		return Document{}, ErrTooLarge
	}
	if !jsonvalue.ValidJSON(string(data)) {
		return Document{}, ErrHelper
	}
	fields, err := decodeObscuraObject(data)
	if err != nil {
		return Document{}, err
	}
	if fields[0] != obscuraProtocol || strings.TrimSpace(fields[2]) == "" {
		return Document{}, ErrUnrecognized
	}
	location, err := whatwg.NewParser().Parse(fields[1])
	if err != nil || !config.PermitsURL(location) {
		return Document{}, ErrDisallowedURL
	}
	return Document{FinalURL: fields[1], Text: fields[2], Format: "html"}, nil
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
	if err == nil && (document.Format != "pdf_text" || url == config.DiscoveryURL) {
		_, err = ParsePage(config, document, utcNow().Format("2006-01-02"), url == config.DiscoveryURL)
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
	_, err = ParsePage(config, document, utcNow().Format("2006-01-02"), url == config.DiscoveryURL)
	if errors.Is(err, ErrUnsupported) {
		return Document{}, originalError
	}
	return document, err
}

// runHelper retains tree ownership until cleanup, including after an apparently successful leader exit.
func runHelper(ctx context.Context, config process.Config, deadline time.Time, output string) (resultValue1 []byte, resultError error) {
	if err := check(ctx, deadline); err != nil {
		return nil, err
	}
	child, err := process.Start(context.Background(), config)
	if err != nil {
		return nil, ErrHelper
	}
	if err := child.Stdin.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "child.Stdin.Close", "error", err)
	}
	defer func() {
		if closeError := child.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close child: %w", closeError))
		}
	}()
	if err := waitForHelper(ctx, child, deadline, output); err != nil {
		return nil, err
	}
	return readHelperOutput(output, deadline)
}

// readHelperOutput bounds file bytes before final deadline admission and closes the file.
func readHelperOutput(output string, deadline time.Time) (resultValue1 []byte, resultError error) {
	root, name, err := rootfile.Parent(output)
	if err != nil {
		return nil, ErrHelper
	}
	defer func() { _ = root.Close() }()
	file, err := root.Open(name)
	if err != nil {
		return nil, ErrHelper
	}
	defer func() {
		if closeError := file.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close file: %w", closeError))
		}
	}()
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

// obscuraEnvironment removes the private-network override using platform-specific name matching.
func obscuraEnvironment() []string {
	environment := []string{}
	for _, entry := range os.Environ() {
		name, _, _ := strings.Cut(entry, "=")
		if name == "OBSCURA_ALLOW_PRIVATE_NETWORK" || runtime.GOOS == "windows" && strings.EqualFold(name, "OBSCURA_ALLOW_PRIVATE_NETWORK") {
			continue
		}
		environment = append(environment, entry)
	}
	return environment
}

func decodeObscuraObject(data []byte) ([]string, error) {
	var document struct {
		Protocol string `json:"protocol"`
		FinalURL string `json:"finalUrl"`
		HTML     string `json:"html"`
	}
	if jsonobject.Decode(data, &document) != nil {
		return nil, ErrHelper
	}
	return []string{document.Protocol, document.FinalURL, document.HTML}, nil
}

// waitForHelper retains tree ownership and gives close failure priority over acquisition errors.
func waitForHelper(ctx context.Context, child *process.Child, deadline time.Time, output string) error {
	for {
		failure := helperAdmissionFailure(ctx, deadline, output)
		if failure != nil {
			if child.Close() != nil {
				return ErrHelper
			}
			return failure
		}
		hasExited, exitError := child.Poll()
		if hasExited {
			if child.Close() != nil || exitError != nil {
				return ErrHelper
			}
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	return nil
}

// helperAdmissionFailure checks cancellation and deadline before inspecting output size.
func helperAdmissionFailure(ctx context.Context, deadline time.Time, output string) error {
	failure := check(ctx, deadline)
	if failure == nil {
		if metadata, err := os.Stat(output); err == nil && metadata.Size() > MaxPageBytes {
			failure = ErrTooLarge
		}
	}
	return failure
}

func removeCaptureDirectory(directory string) error {
	root, name, err := rootfile.Parent(directory)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	return root.RemoveAll(name)
}
