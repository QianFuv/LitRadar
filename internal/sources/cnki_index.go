package sources

import (
	"context"
	"errors"
	"log/slog"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

type clonableCnki[T any] interface {
	cnki.Transport
	Clone() T
}
type cnkiJournalSnapshot struct {
	journal any
	issues  []any
}
type cnkiPageCache struct {
	catalogId, issueId string
	page               cnki.IssueArticlePage
	articles           map[int]*domain.ArticleDraft
}

// CnkiIndexProvider retains frozen journal windows and a bounded reusable detail pool.
type CnkiIndexProvider struct {
	mutex     sync.Mutex
	client    cnki.Transport
	clone     func() cnki.Transport
	pool      *cnkiDetailPool
	snapshots map[string]cnkiJournalSnapshot
	sleep     func(context.Context, time.Duration) error
	isClosed  bool
}

// NewCnkiIndexProvider uses the original single-worker constructor default.
func NewCnkiIndexProvider[T clonableCnki[T]](client T) (*CnkiIndexProvider, error) {
	return NewCnkiIndexProviderWithWorkers(client, 1)
}

// NewCnkiIndexProviderWithWorkers validates concurrency before cloning any transport.
func NewCnkiIndexProviderWithWorkers[T clonableCnki[T]](client T, count int) (*CnkiIndexProvider, error) {
	if count < 1 || count > 32 {
		return nil, &provider.RegistryError{Kind: "InvalidConfiguration", Provider: CnkiProviderName, Detail: "domestic CNKI worker_count must be between 1 and 32"}
	}
	clone := func() cnki.Transport { return client.Clone() }
	return &CnkiIndexProvider{client: client, clone: clone, pool: newCnkiDetailPool(clone, count), snapshots: map[string]cnkiJournalSnapshot{}, sleep: sleepCnkiBatch}, nil
}

// NewCnkiIndexRegistration declares the canonical indexing capability independently of access.
func NewCnkiIndexRegistration[T clonableCnki[T]](client T, count int) (*provider.Registration, error) {
	index, err := NewCnkiIndexProviderWithWorkers(client, count)
	if err != nil {
		return nil, err
	}
	registration, err := provider.NewRegistration(provider.Descriptor{Name: CnkiProviderName, Capabilities: provider.Capabilities{IndexContent: true}}, provider.Implementations{IndexContent: index})
	if err != nil {
		index.Close()
	}
	return registration, err
}

// Close joins owned workers after any active Fetch has completed.
func (index *CnkiIndexProvider) Close() {
	index.mutex.Lock()
	defer index.mutex.Unlock()
	index.pool.close()
	index.isClosed = true
}

func sleepCnkiBatch(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Fetch retries transient batches while retaining successful details only within this invocation.
func (index *CnkiIndexProvider) Fetch(ctx context.Context, catalog domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
	index.mutex.Lock()
	defer index.mutex.Unlock()
	if index.isClosed {
		return domain.ProviderBatch{}, accessFailure(provider.Internal, "domestic CNKI detail worker pool is unavailable")
	}
	attempts := []scholarly.Attempt{}
	defer func() { emitSourceAttemptSummary(ctx, CnkiProviderName, attempts) }()
	var cache *cnkiPageCache
	for attempt := 1; ; attempt++ {
		details := []scholarly.Attempt{}
		batch, err := index.fetchBatch(ctx, catalog, fetch, &details, &cache)
		attempts = append(attempts, index.client.DrainAttempts()...)
		attempts = append(attempts, details...)
		var failure *provider.Error
		if err == nil || attempt >= 3 || !errors.As(err, &failure) || failure.Kind != provider.TemporarilyUnavailable && failure.Kind != provider.InvalidResponse {
			if err == nil && batch.Progress.State == domain.Complete {
				delete(index.snapshots, catalog.CatalogId)
			}
			return batch, err
		}
		slog.InfoContext(ctx, "index.provider.batch.retry", "event", "index.provider.batch.retry", "component", "index", "provider", CnkiProviderName, "failed_attempt", attempt, "next_attempt", attempt+1, "failure_kind", failure.Kind)
		if err := index.client.ResetTransientState(ctx); err != nil {
			return domain.ProviderBatch{}, mapCnkiProviderError(err)
		}
		pool := newCnkiDetailPool(index.clone, index.pool.count)
		index.pool.close()
		index.pool = pool
		if err := index.sleep(ctx, time.Second<<(attempt-1)); err != nil {
			return domain.ProviderBatch{}, mapCnkiProviderError(err)
		}
	}
}
