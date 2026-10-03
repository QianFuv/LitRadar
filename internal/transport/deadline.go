// Package transport preserves source request budgets and bounded decoded responses.
package transport

import (
	"context"
	"errors"
	"time"
)

// LogicalRequestBudget bounds all attempts of one source operation.
const LogicalRequestBudget = 180 * time.Second

// ErrArticleDeadline is the stable public article-access expiration classification.
var ErrArticleDeadline = errors.New("article access deadline expired")

// LogicalDeadline caps a caller's monotonic deadline at the logical request budget.
func LogicalDeadline(caller time.Time) time.Time {
	limit := time.Now().Add(LogicalRequestBudget)
	if !caller.IsZero() && caller.Before(limit) {
		return caller
	}
	return limit
}

// Remaining returns strictly positive time until a finite deadline.
func Remaining(deadline time.Time) (time.Duration, bool) {
	remaining := time.Until(deadline)
	return remaining, remaining > 0
}

// CanWait reports whether a complete delay fits strictly inside a logical deadline.
func CanWait(delay time.Duration, deadline time.Time) bool {
	remaining, hasTime := Remaining(deadline)
	return hasTime && delay < remaining
}

// RetrySleep rejects an oversized server delay immediately instead of retrying early.
func RetrySleep(ctx context.Context, delay time.Duration, deadline time.Time) bool {
	if ctx.Err() != nil || !CanWait(delay, deadline) {
		return false
	}
	if wait(ctx, delay) != nil {
		return false
	}
	_, hasTime := Remaining(deadline)
	return hasTime
}

// EnsureArticleDeadline rejects new work after the optional article deadline.
func EnsureArticleDeadline(deadline time.Time) error {
	if !deadline.IsZero() {
		if _, hasTime := Remaining(deadline); !hasTime {
			return ErrArticleDeadline
		}
	}
	return nil
}

// RequestTimeout clamps a configured timeout to the positive article budget.
func RequestTimeout(configured time.Duration, deadline time.Time) (time.Duration, error) {
	if deadline.IsZero() {
		return configured, nil
	}
	remaining, hasTime := Remaining(deadline)
	if !hasTime {
		return 0, ErrArticleDeadline
	}
	return min(configured, remaining), nil
}

// ArticleSleep consumes the remaining budget before rejecting an oversized delay.
func ArticleSleep(ctx context.Context, delay time.Duration, deadline time.Time) error {
	if !deadline.IsZero() {
		remaining, hasTime := Remaining(deadline)
		if !hasTime {
			return ErrArticleDeadline
		}
		if delay >= remaining {
			if err := wait(ctx, remaining); err != nil {
				return err
			}
			return ErrArticleDeadline
		}
	}
	if err := wait(ctx, delay); err != nil {
		return err
	}
	return EnsureArticleDeadline(deadline)
}

func wait(ctx context.Context, delay time.Duration) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return ctx.Err()
	}
}

var monotonicEpoch = time.Now()

// MonotonicTime advances the initial UTC epoch solely by elapsed monotonic time.
func MonotonicTime() time.Time {
	return monotonicEpoch.Add(time.Since(monotonicEpoch))
}
