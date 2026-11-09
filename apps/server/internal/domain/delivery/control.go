package delivery

import (
	"context"
	"math"
	"sync/atomic"
	"time"
)

const ManualAiRequestBudget uint64 = 8
const ManualJobDeadlineSeconds = 600

// ControlError is a fixed, payload-free execution-stop classification.
type ControlError string

const (
	ControlCancelled        ControlError = "cancelled"
	ControlTimedOut         ControlError = "deadline_exceeded"
	ControlStateUnavailable ControlError = "cancellation_state_unavailable"
	ControlBudgetExhausted  ControlError = "ai_request_budget_exhausted"
)

func (err ControlError) Error() string {
	switch err {
	case ControlCancelled:
		return "Delivery job was cancelled"
	case ControlTimedOut:
		return "Delivery job deadline was exceeded"
	case ControlStateUnavailable:
		return "Delivery job cancellation state is unavailable"
	default:
		return "Delivery AI request budget was exhausted"
	}
}

// ExecutionControl shares one persisted deadline, durable cancellation probe and atomic AI attempt budget.
type ExecutionControl struct {
	deadline  float64
	remaining atomic.Uint64
	probe     func() (bool, error)
	now       func() float64
}

// NewExecutionControl creates a control shared by pointer across endpoints, formats, rounds and summaries.
func NewExecutionControl(deadline float64, budget uint64, probe func() (bool, error)) *ExecutionControl {
	control := &ExecutionControl{deadline: deadline, probe: probe, now: func() float64 { return float64(time.Now().UnixNano()) / 1e9 }}
	control.remaining.Store(budget)
	return control
}

// Deadline returns the original absolute Unix timestamp.
func (control *ExecutionControl) Deadline() float64 { return control.deadline }

// Check probes cancellation before checking the absolute deadline.
func (control *ExecutionControl) Check() error {
	isCancelled, err := control.probe()
	if err != nil {
		return ControlStateUnavailable
	}
	if isCancelled {
		return ControlCancelled
	}
	if control.now() >= control.deadline {
		return ControlTimedOut
	}
	return nil
}

// BeginAiRequest consumes one shared attempt before computing its deadline-capped timeout.
func (control *ExecutionControl) BeginAiRequest(defaultTimeout time.Duration) (time.Duration, error) {
	if err := control.Check(); err != nil {
		return 0, err
	}
	for {
		remaining := control.remaining.Load()
		if remaining == 0 {
			return 0, ControlBudgetExhausted
		}
		if control.remaining.CompareAndSwap(remaining, remaining-1) {
			break
		}
	}
	return control.remainingTimeout(defaultTimeout)
}

// BeginExternalRequest caps non-AI requests without consuming an AI attempt.
func (control *ExecutionControl) BeginExternalRequest(defaultTimeout time.Duration) (time.Duration, error) {
	if err := control.Check(); err != nil {
		return 0, err
	}
	return control.remainingTimeout(defaultTimeout)
}

func (control *ExecutionControl) remainingTimeout(defaultTimeout time.Duration) (time.Duration, error) {
	remaining := control.deadline - control.now()
	if math.IsNaN(remaining) || math.IsInf(remaining, 0) || remaining <= 0 {
		return 0, ControlTimedOut
	}
	bounded := defaultTimeout
	if remaining < defaultTimeout.Seconds() {
		bounded = time.Duration(math.Round(remaining * 1e9))
	}
	return max(time.Millisecond, bounded), nil
}

// Wait checks durable state at most every 25 milliseconds during a retry delay and once after it.
func (control *ExecutionControl) Wait(ctx context.Context, delay time.Duration) error {
	started := time.Now()
	for time.Since(started) < delay {
		if err := control.Check(); err != nil {
			return err
		}
		remaining := max(0, delay-time.Since(started))
		timer := time.NewTimer(min(25*time.Millisecond, remaining))
		select {
		case <-ctx.Done():
			timer.Stop()
			return ControlCancelled
		case <-timer.C:
		}
	}
	return control.Check()
}
