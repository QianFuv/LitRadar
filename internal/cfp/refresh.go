package cfp

import (
	"bytes"
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"
	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
)

var preparedSeed = sync.OnceValues(func() (*storage.PreparedSeed, error) { return storage.PrepareSeed(assets.Seed()) })
var historicalWindowsSeed = sync.OnceValues(func() (*storage.PreparedSeed, error) {
	canonical := bytes.ReplaceAll(assets.Seed(), []byte("\r\n"), []byte("\n"))
	return storage.PrepareSeed(bytes.ReplaceAll(canonical, []byte("\n"), []byte("\r\n")))
})

// EnsureSeed preserves the two historical checkout byte identities without relaxing operator imports.
func EnsureSeed(ctx context.Context, repository *storage.Repository) (storage.ImportResult, error) {
	seed, err := preparedSeed()
	if err != nil {
		return storage.ImportResult{}, &storage.InvalidError{Message: err.Error()}
	}
	result, err := repository.ImportPrepared(ctx, assets.SeedId, seed)
	var invalid *storage.InvalidError
	if !errors.As(err, &invalid) || invalid.Message != "CFP seed identity has different content; use a new import identity" {
		return result, err
	}
	historical, prepareError := historicalWindowsSeed()
	if prepareError != nil {
		return storage.ImportResult{}, &storage.InvalidError{Message: prepareError.Error()}
	}
	return repository.ImportPrepared(ctx, assets.SeedId, historical)
}

type RefreshResult struct {
	SourceKey string  `json:"sourceKey"`
	CatalogId string  `json:"catalogId"`
	Status    string  `json:"status"`
	Notices   uint64  `json:"notices"`
	Error     *string `json:"error"`
}

func failedResult(config SourceConfig, err error) RefreshResult {
	status := "failed"
	if err == ErrUnsupported {
		status = "unsupported"
	}
	message := err.Error()
	return RefreshResult{SourceKey: config.SourceKey, CatalogId: config.CatalogIds[0], Status: status, Error: &message}
}
func leaseSeconds(deadline time.Time) int64 {
	return max(1, min(600, int64(max(0, time.Until(deadline))/time.Second)+1))
}

// RefreshSource publishes only a complete acquisition and preserves retained older notices by exact title.
func RefreshSource(ctx context.Context, repository *storage.Repository, config SourceConfig, transport Transport, deadline time.Time) (RefreshResult, error) {
	databaseCtx := context.WithoutCancel(ctx)
	now := utcNow()
	lease, err := repository.BeginRefresh(databaseCtx, config.SourceKey, now.Unix(), leaseSeconds(deadline))
	if err != nil {
		return RefreshResult{}, err
	}
	acquired, err := Acquire(ctx, transport, config, now.Format("2006-01-02"), deadline)
	if err == nil && !time.Now().Before(deadline) {
		err = ErrDeadline
	}
	if err != nil {
		return failedAcquisitionResult(databaseCtx, repository, lease, config, err)
	}
	if config.RetainsPreviousNotices {
		if err := retainOriginalNotices(databaseCtx, repository, config, &acquired); err != nil {
			return RefreshResult{}, err
		}
	}
	documents := acquisitionDocuments(acquired.Documents)
	capture, err := jsonvalue.EncodeJson(documents)
	if err != nil {
		return RefreshResult{}, &storage.PayloadError{Cause: err}
	}
	if len(capture) > MaxCaptureBytes {
		if err = repository.FailRefresh(databaseCtx, lease, ErrTooLarge.Error(), false); err != nil {
			return RefreshResult{}, err
		}
		return failedResult(config, ErrTooLarge), nil
	}
	publication := storage.Publication{Capture: capture, CaptureFormat: "original_documents_json", SourceUrl: config.DiscoveryUrl, ConfigVersion: config.ConfigVersion, Sources: acquired.Sources, EmptyJournals: acquired.EmptyJournals}
	return publishAcquisition(databaseCtx, repository, lease, config, publication)
}

// publishAcquisition preserves publication failure even when recording that failure also fails.
func publishAcquisition(databaseCtx context.Context, repository *storage.Repository, lease storage.RefreshLease, config SourceConfig, publication storage.Publication) (RefreshResult, error) {
	if err := repository.PublishRefresh(databaseCtx, lease, publication, utcNow().Unix()); err != nil {
		repository.FailRefresh(databaseCtx, lease, "Source publication failed or was superseded", false)
		return RefreshResult{}, err
	}
	return RefreshResult{SourceKey: config.SourceKey, CatalogId: config.CatalogIds[0], Status: "success", Notices: uint64(len(publication.Sources))}, nil
}

func validOptions(options RefreshOptions) bool {
	return options.SourceTimeout > 0 && options.SourceTimeout <= 600*time.Second && options.OverallTimeout > 0 && options.OverallTimeout <= 3600*time.Second
}

// RefreshSources runs two independent sources and returns results in registration order.
func RefreshSources(ctx context.Context, repository *storage.Repository, configs []SourceConfig, options RefreshOptions) ([]RefreshResult, error) {
	if !validOptions(options) {
		return nil, &storage.InvalidError{Message: "Invalid CFP acquisition time budget"}
	}
	if _, err := EnsureSeed(context.WithoutCancel(ctx), repository); err != nil {
		return nil, err
	}
	deadline := time.Now().Add(options.OverallTimeout)
	results := make([]RefreshResult, len(configs))
	failures := make([]error, len(configs))
	var next atomic.Int64
	var workers sync.WaitGroup
	for worker := 0; worker < 2; worker++ {
		workers.Go(func() {
			for {
				index := int(next.Add(1) - 1)
				if index >= len(configs) {
					return
				}
				config := configs[index]
				if !time.Now().Before(deadline) || ctx.Err() != nil {
					reason := "Batch deadline or cancellation prevented this attempt"
					results[index] = RefreshResult{SourceKey: config.SourceKey, CatalogId: config.CatalogIds[0], Status: "not_attempted", Error: &reason}
					continue
				}
				transport := NewLiveTransport(options)
				sourceDeadline := time.Now().Add(options.SourceTimeout)
				if sourceDeadline.After(deadline) {
					sourceDeadline = deadline
				}
				results[index], failures[index] = RefreshSource(ctx, repository, config, transport, sourceDeadline)
				transport.Close()
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

// retainOriginalNotices appends older exact-title omissions in stored order and clears empty declarations.
func retainOriginalNotices(databaseCtx context.Context, repository *storage.Repository, config SourceConfig, acquired *Acquisition) error {
	originals, err := repository.LoadOriginals(databaseCtx, config.SourceKey)
	if err != nil {
		return err
	}
	for _, original := range originals {
		hasTitle := false
		for _, source := range acquired.Sources {
			if source.Title == original.Title {
				hasTitle = true
				break
			}
		}
		if !hasTitle {
			acquired.Sources = append(acquired.Sources, original)
		}
	}
	if len(acquired.Sources) > 0 {
		acquired.EmptyJournals = []domain.EmptyJournal{}
	}
	return nil
}

// acquisitionDocuments preserves capture document order and original publisher text.
func acquisitionDocuments(acquired []Document) []map[string]string {
	documents := []map[string]string{}
	for _, document := range acquired {
		documents = append(documents, map[string]string{"url": document.FinalUrl, "format": document.Format, "text": document.Text})
	}
	return documents
}

// failedAcquisitionResult records acquisition failure before returning its original source status.
func failedAcquisitionResult(databaseCtx context.Context, repository *storage.Repository, lease storage.RefreshLease, config SourceConfig, err error) (RefreshResult, error) {
	if failure := repository.FailRefresh(databaseCtx, lease, err.Error(), err == ErrUnsupported); failure != nil {
		return RefreshResult{}, failure
	}
	return failedResult(config, err), nil
}
