package delivery

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestOneBudgetSurvivesConcurrentEndpointAndFormatAttempts(t *testing.T) {
	control := NewExecutionControl(100, ManualAiRequestBudget, func() (bool, error) { return false, nil })
	control.now = func() float64 { return 1 }
	var successes atomic.Int32
	var group sync.WaitGroup
	for range 20 {
		group.Add(1)
		go func() {
			defer group.Done()
			_, err := control.BeginAiRequest(time.Second)
			if err == nil {
				successes.Add(1)
			} else if err != ControlBudgetExhausted {
				t.Error(err)
			}
		}()
	}
	group.Wait()
	if successes.Load() != 8 {
		t.Fatalf("attempts=%d", successes.Load())
	}
	if _, err := control.BeginExternalRequest(time.Second); err != nil {
		t.Fatalf("PushPlus must not consume AI budget: %v", err)
	}
}

func TestCancellationPrecedesDeadlineAndBudget(t *testing.T) {
	probeError := errors.New("private database failure")
	for _, test := range []struct {
		cancel   bool
		failure  error
		expected error
	}{{true, nil, ControlCancelled}, {false, probeError, ControlStateUnavailable}, {false, nil, ControlTimedOut}} {
		control := NewExecutionControl(1, 0, func() (bool, error) { return test.cancel, test.failure })
		control.now = func() float64 { return 2 }
		if _, err := control.BeginAiRequest(time.Second); err != test.expected {
			t.Fatalf("got %v want %v", err, test.expected)
		}
	}
}

func TestTimeoutAfterReservationDoesNotRefundAttempt(t *testing.T) {
	control := NewExecutionControl(2, 1, func() (bool, error) { return false, nil })
	calls := 0
	control.now = func() float64 {
		calls++
		if calls == 1 {
			return 1
		}
		return 2
	}
	if _, err := control.BeginAiRequest(time.Second); err != ControlTimedOut {
		t.Fatal(err)
	}
	control.now = func() float64 { return 1 }
	if _, err := control.BeginAiRequest(time.Second); err != ControlBudgetExhausted {
		t.Fatal(err)
	}
	control = NewExecutionControl(2, 1, func() (bool, error) { return false, nil })
	control.now = func() float64 { return 1.9999 }
	if timeout, err := control.BeginAiRequest(time.Second); err != nil || timeout != time.Millisecond {
		t.Fatalf("minimum timeout: %v %v", timeout, err)
	}
}

func TestRetryWaitChecksDurableCancellation(t *testing.T) {
	var calls atomic.Int32
	control := NewExecutionControl(100, 8, func() (bool, error) { return calls.Add(1) > 1, nil })
	control.now = func() float64 { return 1 }
	started := time.Now()
	if err := control.Wait(context.Background(), time.Second); err != ControlCancelled {
		t.Fatal(err)
	}
	if time.Since(started) > 500*time.Millisecond {
		t.Fatal("cancellation was not polled")
	}
}
