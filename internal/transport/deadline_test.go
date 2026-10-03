package transport

import (
	"context"
	"errors"
	"math"
	"net/http"
	"testing"
	"time"
)

func TestLogicalBudgetAndDistinctWaitPolicies(t *testing.T) {
	started := time.Now()
	early := started.Add(time.Second)
	if LogicalDeadline(early) != early {
		t.Fatal("lost caller deadline")
	}
	capped := LogicalDeadline(started.Add(600 * time.Second))
	if capped.Before(started.Add(180*time.Second)) || capped.After(started.Add(181*time.Second)) {
		t.Fatal(capped)
	}
	if RetrySleep(context.Background(), 300*time.Second, time.Now().Add(30*time.Second)) {
		t.Fatal("oversized delay accepted")
	}
	if time.Since(started) > time.Second {
		t.Fatal("logical retry consumed budget")
	}
	started = time.Now()
	if err := ArticleSleep(context.Background(), time.Second, started.Add(25*time.Millisecond)); err != ErrArticleDeadline {
		t.Fatal(err)
	}
	if elapsed := time.Since(started); elapsed < 15*time.Millisecond || elapsed > 500*time.Millisecond {
		t.Fatal(elapsed)
	}
	if CanWait(time.Second, time.Now().Add(time.Second)) {
		t.Fatal("equal delay must not fit")
	}
	if _, hasTime := Remaining(time.Now()); hasTime {
		t.Fatal("zero remaining accepted")
	}
}

func TestExpiredDeadlineAndCancellationRejectWork(t *testing.T) {
	deadline := time.Now().Add(-time.Second)
	if EnsureArticleDeadline(deadline) != ErrArticleDeadline {
		t.Fatal("expired deadline accepted")
	}
	if _, err := RequestTimeout(30*time.Second, deadline); err != ErrArticleDeadline {
		t.Fatal(err)
	}
	if timeout, err := RequestTimeout(30*time.Second, time.Time{}); err != nil || timeout != 30*time.Second {
		t.Fatal(timeout, err)
	}
	if timeout, err := RequestTimeout(30*time.Second, time.Now().Add(100*time.Millisecond)); err != nil || timeout <= 0 || timeout > 100*time.Millisecond {
		t.Fatal(timeout, err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if RetrySleep(ctx, 0, time.Now().Add(time.Second)) {
		t.Fatal("cancelled retry accepted")
	}
	if err := ArticleSleep(ctx, time.Hour, time.Time{}); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestHeaderVisibilityFirstValueAndWideDelay(t *testing.T) {
	for _, value := range []string{"\u00a012", "12\r", "12\n", "12\x7f"} {
		if _, isValid := RetryAfter(http.Header{"Retry-After": []string{value}}); isValid {
			t.Fatalf("invalid visible header %q", value)
		}
	}
	delay, isValid := RetryAfter(http.Header{"Retry-After": []string{"\t12\t", "1"}})
	if !isValid || delay.Seconds != 12 {
		t.Fatal(delay, isValid)
	}
	delay, isValid = ParseRetryAfter("18446744073709551615", time.Now())
	if !isValid || delay.Seconds != math.MaxUint64 || delay.Duration() != time.Duration(math.MaxInt64) {
		t.Fatal(delay, isValid)
	}
	if RetrySleep(context.Background(), delay.Duration(), time.Now().Add(time.Second)) {
		t.Fatal("overflow caused early retry")
	}
	first := MonotonicTime()
	second := MonotonicTime()
	if second.Before(first) {
		t.Fatal("monotonic epoch regressed")
	}
}
