package sources

import (
	"context"
	"log/slog"
	"slices"
	"sync"
	"sync/atomic"

	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

type cnkiDetailTask struct {
	ordinal    int
	summary    any
	url        string
	platformId *string
}
type cnkiDetailOutcome struct {
	task             cnkiDetailTask
	detail           any
	err, errorWorker error
	attempts         []scholarly.Attempt
}
type cnkiDetailJob struct {
	ctx     context.Context
	task    cnkiDetailTask
	results chan<- cnkiDetailOutcome
}
type cnkiDetailPool struct {
	jobs         chan cnkiDetailJob
	workers      sync.WaitGroup
	count        int
	active, peak atomic.Int64
}

func newCnkiDetailPool(clone func() cnki.Transport, count int) *cnkiDetailPool {
	pool := &cnkiDetailPool{jobs: make(chan cnkiDetailJob, count), count: count}
	for range count {
		client := clone()
		pool.workers.Go(func() {
			for job := range pool.jobs {
				job.results <- pool.run(client, job)
			}
		})
	}
	return pool
}

func (pool *cnkiDetailPool) close() {
	if pool.jobs != nil {
		close(pool.jobs)
		pool.workers.Wait()
		pool.jobs = nil
	}
}

func (pool *cnkiDetailPool) run(client cnki.Transport, job cnkiDetailJob) (outcome cnkiDetailOutcome) {
	outcome.task = job.task
	defer func() {
		if recover() != nil {
			outcome.errorWorker = accessFailure(provider.Internal, "domestic CNKI detail worker panicked")
		}
	}()
	client.DrainAttempts()
	active := pool.active.Add(1)
	defer pool.active.Add(-1)
	for {
		peak := pool.peak.Load()
		if active <= peak || pool.peak.CompareAndSwap(peak, active) {
			break
		}
	}
	outcome.detail, outcome.err = client.ArticleDetail(job.ctx, job.task.url, job.task.platformId)
	outcome.attempts = client.DrainAttempts()
	return outcome
}

func (pool *cnkiDetailPool) execute(ctx context.Context, tasks []cnkiDetailTask) ([]cnkiDetailOutcome, error) {
	if len(tasks) == 0 {
		return []cnkiDetailOutcome{}, nil
	}
	if pool.jobs == nil {
		return nil, accessFailure(provider.Internal, "domestic CNKI detail worker pool is unavailable")
	}
	results := make(chan cnkiDetailOutcome, len(tasks))
	for _, task := range tasks {
		pool.jobs <- cnkiDetailJob{ctx, task, results}
	}
	outcomes := make([]cnkiDetailOutcome, 0, len(tasks))
	var firstError error
	for range tasks {
		outcome := <-results
		if outcome.errorWorker != nil {
			if firstError == nil {
				firstError = outcome.errorWorker
			}
		} else {
			outcomes = append(outcomes, outcome)
		}
	}
	slog.InfoContext(ctx, "index.provider.concurrency", "event", "index.provider.concurrency", "component", "index", "provider", CnkiProviderName, "configured_workers", pool.count, "effective_workers", min(pool.count, len(tasks)), "worker_threads_created", pool.count, "active_detail_requests", pool.active.Load(), "peak_detail_requests", pool.peak.Load(), "aggregate_limit", 32)
	if firstError != nil {
		return nil, firstError
	}
	slices.SortFunc(outcomes, func(first, last cnkiDetailOutcome) int { return first.task.ordinal - last.task.ordinal })
	return outcomes, nil
}
