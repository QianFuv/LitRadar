package admission

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"time"
)

func TestCancellationDoesNotReleaseRunningCapacity(t *testing.T) {
	gate := New(1)
	ctx, cancel := context.WithCancel(context.Background())
	started := make(chan struct{})
	finish := make(chan struct{})
	firstResult := make(chan error, 1)
	go func() {
		_, err := Run(ctx, gate, func() (int, error) { close(started); <-finish; return 1, nil })
		firstResult <- err
	}()
	<-started
	cancel()
	if err := <-firstResult; !errors.Is(err, context.Canceled) {
		t.Fatalf("Caller did not cancel: %v", err)
	}
	var executions atomic.Int32
	queued, cancelQueued := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancelQueued()
	_, err := Run(queued, gate, func() (int, error) { executions.Add(1); return 2, nil })
	if !errors.Is(err, context.DeadlineExceeded) || executions.Load() != 0 {
		t.Fatal("Cancelled worker released capacity before actual completion")
	}
	close(finish)
	remaining, cancelRemaining := context.WithTimeout(context.Background(), time.Second)
	defer cancelRemaining()
	value, err := Run(remaining, gate, func() (int, error) { return 3, nil })
	if err != nil || value != 3 {
		t.Fatalf("Completed worker did not release capacity: %d %v", value, err)
	}
}
