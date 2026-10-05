package executor

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/admission"
)

func TestQueueDeadlineDoesNotCancelAdmittedWork(t *testing.T) {
	pool := New(1, 10*time.Millisecond)
	value, err := Run(context.Background(), pool, func() (int, error) {
		time.Sleep(40 * time.Millisecond)
		return 42, nil
	})
	if err != nil || value != 42 {
		t.Fatalf("admitted work: %d %v", value, err)
	}
}

func TestCancelledRequestRetainsCapacityAndCloseWakesWaiters(t *testing.T) {
	pool := New(1, time.Second)
	requestContext, cancel := context.WithCancel(context.Background())
	started, finish, completed := make(chan struct{}), make(chan struct{}), make(chan struct{})
	result := make(chan error, 1)
	go func() {
		_, err := Run(requestContext, pool, func() (int, error) {
			close(started)
			<-finish
			close(completed)
			return 1, nil
		})
		result <- err
	}()
	<-started
	cancel()
	defer close(finish)
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	_, err := RunWithQueueTimeout(context.Background(), pool, 10*time.Millisecond, func() (int, error) {
		t.Error("cancelled request released active worker capacity")
		return 0, nil
	})
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatal(err)
	}
	go func() {
		_, err := Run(context.Background(), pool, func() (int, error) {
			t.Error("closed pool admitted work")
			return 0, nil
		})
		result <- err
	}()
	pool.Close()
	pool.Close()
	if err := <-result; !errors.Is(err, admission.ErrClosed) {
		t.Fatal(err)
	}
	select {
	case <-completed:
		t.Fatal("shutdown cancelled active work")
	default:
	}
}

func TestWorkerPanicIsPrivateAndReleasesCapacity(t *testing.T) {
	pool := New(1, time.Second)
	_, err := Run(context.Background(), pool, func() (int, error) { panic("secret fixture") })
	if !errors.Is(err, ErrWorkerFailed) {
		t.Fatalf("panic leaked: %v", err)
	}
	value, err := Run(context.Background(), pool, func() (int, error) { return 7, nil })
	if err != nil || value != 7 {
		t.Fatalf("capacity leaked: %d %v", value, err)
	}
}
