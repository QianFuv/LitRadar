// Package admission bounds non-cancellable work independently of request cancellation.
package admission

import (
	"context"
	"errors"
	"sync"
)

// ErrClosed means that shutdown stopped admission of new work.
var ErrClosed = errors.New("admission gate closed")

// Gate retains active capacity until the actual worker returns.
type Gate struct {
	permits   chan struct{}
	closed    chan struct{}
	closeOnce sync.Once
	mutex     sync.Mutex
	active    sync.WaitGroup
}

// New constructs a fixed positive work limit.
func New(capacity int) *Gate {
	if capacity < 1 {
		panic("admission capacity must be positive")
	}
	return &Gate{permits: make(chan struct{}, capacity), closed: make(chan struct{})}
}

// Close rejects queued and subsequent work without releasing running capacity.
func (gate *Gate) Close() {
	gate.mutex.Lock()
	defer gate.mutex.Unlock()
	gate.closeOnce.Do(func() { close(gate.closed) })
}

// Wait closes admission and retains ownership until every admitted worker has returned.
func (gate *Gate) Wait() { gate.Close(); gate.active.Wait() }

// Run removes cancelled waiters, but never releases capacity while work is still executing.
func Run[Value any](ctx context.Context, gate *Gate, work func() (Value, error)) (Value, error) {
	return RunQueued(ctx, ctx, gate, work)
}

// RunQueued applies queue cancellation only until admission; caller cancellation
// stops waiting for the result but retains capacity until the worker returns.
func RunQueued[Value any](ctx, queueContext context.Context, gate *Gate, work func() (Value, error)) (Value, error) {
	var zero Value
	select {
	case gate.permits <- struct{}{}:
	case <-gate.closed:
		return zero, ErrClosed
	case <-queueContext.Done():
		return zero, queueContext.Err()
	case <-ctx.Done():
		return zero, ctx.Err()
	}
	if err := queueContext.Err(); err != nil {
		<-gate.permits
		return zero, err
	}
	if err := ctx.Err(); err != nil {
		<-gate.permits
		return zero, err
	}
	gate.mutex.Lock()
	select {
	case <-gate.closed:
		gate.mutex.Unlock()
		<-gate.permits
		return zero, ErrClosed
	default:
	}
	gate.active.Add(1)
	gate.mutex.Unlock()
	type result struct {
		value Value
		err   error
	}
	completed := make(chan result, 1)
	go func() {
		defer gate.active.Done()
		defer func() { <-gate.permits }()
		value, err := work()
		completed <- result{value, err}
	}()
	select {
	case outcome := <-completed:
		return outcome.value, outcome.err
	case <-ctx.Done():
		return zero, ctx.Err()
	}
}
