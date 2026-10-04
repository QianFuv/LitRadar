package cfp

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	jsoncompat "github.com/QianFuv/LitRadar/internal/domain/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

type FullTextResult struct {
	SourceKey  string   `json:"sourceKey"`
	CatalogId  string   `json:"catalogId"`
	Status     string   `json:"status"`
	Notices    uint64   `json:"notices"`
	Recovered  uint64   `json:"recovered"`
	Updated    uint64   `json:"updated"`
	Unresolved []string `json:"unresolved"`
	Error      *string  `json:"error"`
}

// EvidenceError retains a completed publication when writing its separate recovery evidence fails.
type EvidenceError struct {
	Result FullTextResult
	Cause  error
}

func (failure *EvidenceError) Error() string { return failure.Cause.Error() }
func (failure *EvidenceError) Unwrap() error { return failure.Cause }

type capturedDocument struct {
	document Document
	err      error
}
type captureTransport interface {
	httpDocument(context.Context, SourceConfig, string, time.Time) (Document, error)
	obscuraDocument(context.Context, SourceConfig, string, time.Time) (Document, error)
	Close()
}
type captureCache struct {
	documents       map[string]capturedDocument
	browserAttempts map[string]bool
	bytes           int
	create          func(RefreshOptions) captureTransport
}

func newCaptureCache() *captureCache {
	return &captureCache{documents: map[string]capturedDocument{}, browserAttempts: map[string]bool{}, create: func(options RefreshOptions) captureTransport { return NewLiveTransport(options) }}
}

func (cache *captureCache) capture(ctx context.Context, config SourceConfig, value string, options RefreshOptions, deadline time.Time, shouldUseBrowser bool) (Document, error) {
	if !shouldUseBrowser || cache.browserAttempts[value] {
		if captured, has := cache.documents[value]; has {
			return captured.document, captured.err
		}
	}
	if !time.Now().Before(deadline) || ctx.Err() != nil {
		return Document{}, ErrDeadline
	}
	location, err := whatwg.NewParser().Parse(value)
	if err != nil || location.Hostname() == "" {
		return Document{}, ErrDisallowedUrl
	}
	config.AllowedUrls = append(append([]UrlRule{}, config.AllowedUrls...), UrlRule{Host: location.Hostname(), PathPrefix: location.Pathname()})
	shouldUseBrowser = shouldUseBrowser || location.Hostname() == "link.springer.com"
	transport := cache.create(options)
	defer transport.Close()
	url := location.Href(false)
	var document Document
	if shouldUseBrowser {
		cache.browserAttempts[url] = true
		document, err = transport.obscuraDocument(ctx, config, url, deadline)
	} else {
		document, err = transport.httpDocument(ctx, config, url, deadline)
	}
	if err == nil && IsChallenge(document) {
		err = ErrChallenge
	}
	if !shouldUseBrowser && canRender(err, false) {
		cache.browserAttempts[url] = true
		document, err = transport.obscuraDocument(ctx, config, url, deadline)
	}
	if err == nil {
		if IsChallenge(document) {
			err = ErrChallenge
		} else if cache.bytes+len(document.Text) > MaxCaptureBytes {
			err = ErrTooLarge
		} else {
			cache.bytes += len(document.Text)
		}
	}
	cache.documents[url] = capturedDocument{document: document, err: err}
	return document, err
}

func recoverOriginal(ctx context.Context, original domain.Source, config SourceConfig, cache *captureCache, options RefreshOptions, deadline time.Time) (domain.Source, error) {
	type visit struct {
		url   string
		depth int
	}
	pending := []visit{{original.SourceUrl, 0}}
	visited := map[string]bool{}
	var lastError error = ErrUnrecognized
	for len(pending) > 0 {
		current := pending[0]
		pending = pending[1:]
		if !time.Now().Before(deadline) {
			return domain.Source{}, ErrDeadline
		}
		if len(visited) >= 8 {
			break
		}
		if visited[current.url] {
			continue
		}
		visited[current.url] = true
		document, err := cache.capture(ctx, config, current.url, options, deadline, false)
		if err != nil {
			lastError = err
			continue
		}
		if source, err := ExtractFullText(original, document); err == nil {
			return source, nil
		}
		links := OriginalLinks(original, document)
		if len(links) == 0 && document.Format == "html" && !cache.browserAttempts[current.url] {
			if rendered, err := cache.capture(ctx, config, current.url, options, deadline, true); err == nil {
				if source, err := ExtractFullText(original, rendered); err == nil {
					return source, nil
				}
				links = OriginalLinks(original, rendered)
			}
		}
		if current.depth < 2 {
			for _, link := range links[:min(len(links), 4)] {
				pending = append(pending, visit{link, current.depth + 1})
			}
		}
	}
	return domain.Source{}, lastError
}

func (cache *captureCache) resume(path, sourceKey string) error {
	metadata, err := os.Stat(path)
	if err != nil || !metadata.Mode().IsRegular() {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return &storage.InvalidError{Message: fmt.Sprintf("Could not read the saved capture: %v", err)}
	}
	if !jsoncompat.ValidJson(string(data)) {
		return &storage.PayloadError{Cause: fmt.Errorf("invalid saved capture JSON")}
	}
	var saved map[string]json.RawMessage
	json.Unmarshal(data, &saved)
	var result map[string]json.RawMessage
	json.Unmarshal(saved["result"], &result)
	var key *string
	json.Unmarshal(result["sourceKey"], &key)
	if key == nil || *key != sourceKey {
		return &storage.InvalidError{Message: "Saved capture journal identity does not match"}
	}
	var documents []json.RawMessage
	json.Unmarshal(saved["documents"], &documents)
	for _, raw := range documents {
		var fields map[string]json.RawMessage
		if json.Unmarshal(raw, &fields) != nil {
			continue
		}
		var requested, url, text, format *string
		json.Unmarshal(fields["requestedUrl"], &requested)
		json.Unmarshal(fields["url"], &url)
		json.Unmarshal(fields["text"], &text)
		json.Unmarshal(fields["format"], &format)
		if requested == nil || url == nil || text == nil || format == nil {
			continue
		}
		cache.bytes += len(*text)
		if cache.bytes > MaxCaptureBytes {
			return &storage.InvalidError{Message: "Saved capture exceeds the source budget"}
		}
		captured := capturedDocument{document: Document{FinalUrl: *url, Text: *text, Format: *format}}
		cache.documents[*requested] = captured
		cache.documents[*url] = captured
		if strings.HasPrefix(*url, "https://link.springer.com/") {
			cache.browserAttempts[*requested] = true
		}
	}
	var attempts []json.RawMessage
	json.Unmarshal(saved["browserAttempts"], &attempts)
	for _, raw := range attempts {
		var url *string
		if json.Unmarshal(raw, &url) == nil && url != nil {
			cache.browserAttempts[*url] = true
		}
	}
	return nil
}
func sortedMapKeys[V any](values map[string]V) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	return keys
}
func (cache *captureCache) evidence() []map[string]string {
	documents := []map[string]string{}
	for _, url := range sortedMapKeys(cache.documents) {
		captured := cache.documents[url]
		if captured.err != nil {
			documents = append(documents, map[string]string{"requestedUrl": url, "error": captured.err.Error()})
		} else {
			documents = append(documents, map[string]string{"requestedUrl": url, "url": captured.document.FinalUrl, "format": captured.document.Format, "text": captured.document.Text})
		}
	}
	return documents
}

// RefreshFullTextSource supplements saved notices independently of automatic discovery support.
func RefreshFullTextSource(ctx context.Context, repository *storage.Repository, config SourceConfig, options RefreshOptions, deadline time.Time, captureDirectory *string, shouldResume bool) (FullTextResult, error) {
	databaseCtx := context.WithoutCancel(ctx)
	originals, err := repository.LoadOriginals(databaseCtx, config.SourceKey)
	if err != nil {
		return FullTextResult{}, err
	}
	result := FullTextResult{SourceKey: config.SourceKey, CatalogId: config.CatalogIds[0], Status: "no_notices", Notices: uint64(len(originals)), Unresolved: []string{}}
	if len(originals) == 0 {
		return result, nil
	}
	if !time.Now().Before(deadline) || ctx.Err() != nil {
		result.Status = "not_attempted"
		return result, nil
	}
	lease, err := repository.BeginRefresh(databaseCtx, config.SourceKey, utcNow().Unix(), leaseSeconds(deadline))
	if err != nil {
		return FullTextResult{}, err
	}
	acquisitionDeadline := deadline.Add(-5 * time.Second)
	originals, err = repository.LoadOriginals(databaseCtx, config.SourceKey)
	if err != nil {
		return FullTextResult{}, err
	}
	result.Notices = uint64(len(originals))
	cache := newCaptureCache()
	capturePath := ""
	if captureDirectory != nil {
		name := strings.Map(func(character rune) rune {
			if character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '-' {
				return character
			}
			return '_'
		}, config.SourceKey)
		capturePath = filepath.Join(*captureDirectory, name+".json")
	}
	if shouldResume && capturePath != "" {
		if err = cache.resume(capturePath, config.SourceKey); err != nil {
			return FullTextResult{}, err
		}
	}
	sources := []domain.Source{}
	for _, original := range originals {
		source, err := recoverOriginal(ctx, original, config, cache, options, acquisitionDeadline)
		if err == nil {
			result.Recovered++
			sources = append(sources, source)
		} else {
			result.Unresolved = append(result.Unresolved, original.Title)
			message := err.Error()
			result.Error = &message
			sources = append(sources, original)
		}
	}
	documents := cache.evidence()
	capture, err := jsoncompat.EncodeJson(documents)
	if err != nil {
		return FullTextResult{}, &storage.PayloadError{Cause: err}
	}
	if result.Recovered > 0 && len(capture) <= MaxCaptureBytes && ctx.Err() == nil {
		publication := storage.Publication{Capture: capture, CaptureFormat: "original_documents_json", SourceUrl: config.DiscoveryUrl, ConfigVersion: config.ConfigVersion, Sources: sources, EmptyJournals: []domain.EmptyJournal{}}
		err = repository.PublishFullText(databaseCtx, lease, publication, utcNow().Unix(), uint64(len(result.Unresolved)))
		if err == nil {
			result.Updated = result.Recovered
			result.Status = "success"
			if len(result.Unresolved) > 0 {
				result.Status = "partial"
			}
		} else {
			result.Status = "failed"
			message := err.Error()
			result.Error = &message
			if err = repository.FailRefresh(databaseCtx, lease, "Full-text publication failed", false); err != nil {
				return FullTextResult{}, err
			}
		}
	} else {
		result.Status = "failed"
		if err = repository.FailRefresh(databaseCtx, lease, "Complete original text could not be verified for every notice", false); err != nil {
			return FullTextResult{}, err
		}
	}
	if capturePath != "" {
		evidence, err := jsoncompat.EncodeJson(map[string]any{"result": result, "sources": sources, "documents": documents, "browserAttempts": sortedMapKeys(cache.browserAttempts)})
		if err != nil {
			return FullTextResult{}, &storage.PayloadError{Cause: err}
		}
		if err = os.WriteFile(capturePath, []byte(evidence), 0600); err != nil {
			return result, &EvidenceError{Result: result, Cause: &storage.InvalidError{Message: fmt.Sprintf("Could not save full-text capture evidence: %v", err)}}
		}
	}
	slog.InfoContext(ctx, "CFP full-text refresh completed", "event", "cfp.full_text.completed", "catalog_id", result.CatalogId, "status", result.Status, "recovered", result.Recovered, "notices", result.Notices)
	return result, nil
}

// RefreshFullTexts uses four source workers and preserves input ordering after all workers finish.
func RefreshFullTexts(ctx context.Context, repository *storage.Repository, configs []SourceConfig, options RefreshOptions, captureDirectory *string, shouldResume bool) ([]FullTextResult, error) {
	if !validOptions(options) {
		return nil, &storage.InvalidError{Message: "Invalid CFP full-text acquisition time budget"}
	}
	if captureDirectory != nil {
		if err := os.MkdirAll(*captureDirectory, 0700); err != nil {
			return nil, &storage.InvalidError{Message: fmt.Sprintf("Could not create full-text capture directory: %v", err)}
		}
	}
	if _, err := EnsureSeed(context.WithoutCancel(ctx), repository); err != nil {
		return nil, err
	}
	deadline := time.Now().Add(options.OverallTimeout)
	results := make([]FullTextResult, len(configs))
	failures := make([]error, len(configs))
	var next atomic.Int64
	var workers sync.WaitGroup
	for worker := 0; worker < 4; worker++ {
		workers.Go(func() {
			for {
				index := int(next.Add(1) - 1)
				if index >= len(configs) {
					return
				}
				sourceDeadline := time.Now().Add(options.SourceTimeout)
				if sourceDeadline.After(deadline) {
					sourceDeadline = deadline
				}
				results[index], failures[index] = RefreshFullTextSource(ctx, repository, configs[index], options, sourceDeadline, captureDirectory, shouldResume)
			}
		})
	}
	workers.Wait()
	for _, err := range failures {
		if err != nil {
			return nil, err
		}
	}
	return results, nil
}
