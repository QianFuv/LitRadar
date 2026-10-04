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
}

// New constructs a fixed positive work limit.
func New(capacity int) *Gate {
	if capacity < 1 {
		panic("admission capacity must be positive")
	}
	return &Gate{permits: make(chan struct{}, capacity), closed: make(chan struct{})}
}

// Close rejects queued and subsequent work without releasing running capacity.
func (gate *Gate) Close() { gate.closeOnce.Do(func() { close(gate.closed) }) }

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
	select {
	case <-gate.closed:
		<-gate.permits
		return zero, ErrClosed
	default:
	}
	if err := queueContext.Err(); err != nil {
		<-gate.permits
		return zero, err
	}
	if err := ctx.Err(); err != nil {
		<-gate.permits
		return zero, err
	}
	type result struct {
		value Value
		err   error
	}
	completed := make(chan result, 1)
	go func() {
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
