// Package admission bounds non-cancellable work independently of request cancellation.
package admission

import "context"

// Gate retains active capacity until the actual worker returns.
type Gate struct{ permits chan struct{} }

// New constructs a fixed positive work limit.
func New(capacity int) *Gate {
	if capacity < 1 {
		panic("admission capacity must be positive")
	}
	return &Gate{permits: make(chan struct{}, capacity)}
}

// Run removes cancelled waiters, but never releases capacity while work is still executing.
func Run[Value any](ctx context.Context, gate *Gate, work func() (Value, error)) (Value, error) {
	var zero Value
	select {
	case gate.permits <- struct{}{}:
	case <-ctx.Done():
		return zero, ctx.Err()
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
