// Package executor isolates storage, upstream and password work behind queue-only deadlines.
package executor

import (
	"context"
	"errors"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/runtime/observability"
)

// ErrWorkerFailed hides panic contents at public API boundaries.
var ErrWorkerFailed = errors.New("blocking task failed to join")

// Pool bounds active workers independently from requests waiting for their result.
type Pool struct {
	gate         *admission.Gate
	queueTimeout time.Duration
}

// New creates a worker pool with a separate admission deadline.
func New(capacity int, queueTimeout time.Duration) *Pool {
	return &Pool{gate: admission.New(capacity), queueTimeout: queueTimeout}
}

// Close wakes queued callers and rejects new work while active workers finish.
func (pool *Pool) Close() { pool.gate.Close() }

// Wait closes admission and waits for actual workers before the host releases borrowed resources.
func (pool *Pool) Wait() { pool.gate.Wait() }

// Run bounds queue time and converts worker panics into a safe executor failure.
// Work must own its captured inputs. Reads may use ctx; admitted writes must
// choose an operation context that survives cancellation of the waiting caller.
func Run[Value any](ctx context.Context, pool *Pool, work func() (Value, error)) (Value, error) {
	return RunWithQueueTimeout(ctx, pool, pool.queueTimeout, work)
}

// RunWithQueueTimeout supports readiness and upstream operations with a smaller admission budget.
func RunWithQueueTimeout[Value any](ctx context.Context, pool *Pool, timeout time.Duration, work func() (Value, error)) (Value, error) {
	queueContext, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	return admission.RunQueued(ctx, queueContext, pool.gate, func() (value Value, err error) {
		defer func() {
			if recover() != nil {
				observability.ReportPanic(ctx)
				var zero Value
				value, err = zero, ErrWorkerFailed
			}
		}()
		return work()
	})
}
