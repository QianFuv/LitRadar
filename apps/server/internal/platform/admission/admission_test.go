package admission

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
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

func TestShutdownWaitRetainsCancelledWorkerOwnership(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := New(1)
		ctx, cancel := context.WithCancel(t.Context())
		started, release, callerDone := make(chan struct{}), make(chan struct{}), make(chan struct{})
		go func() {
			defer close(callerDone)
			_, _ = Run(ctx, gate, func() (int, error) { close(started); <-release; return 1, nil })
		}()
		<-started
		cancel()
		<-callerDone
		gate.Close()
		waited := make(chan struct{})
		go func() { gate.Wait(); close(waited) }()
		synctest.Wait()
		select {
		case <-waited:
			t.Fatal("shutdown released resources while cancelled work still used them")
		default:
		}
		close(release)
		<-waited
		if _, err := Run(t.Context(), gate, func() (int, error) { t.Error("work admitted after shutdown"); return 0, nil }); !errors.Is(err, ErrClosed) {
			t.Fatal(err)
		}

	})
}
func TestConcurrentShutdownCannotRaceNewWorkerOwnership(t *testing.T) {
	for range 50 {
		gate := New(4)
		start := make(chan struct{})
		var callers sync.WaitGroup
		var active atomic.Int32
		for range 12 {
			callers.Go(func() {
				<-start
				_, _ = Run(context.Background(), gate, func() (int, error) { active.Add(1); defer active.Add(-1); time.Sleep(time.Millisecond); return 1, nil })
			})
		}
		close(start)
		gate.Wait()
		callers.Wait()
		if active.Load() != 0 {
			t.Fatal("shutdown lost an admitted worker")
		}
	}
}
