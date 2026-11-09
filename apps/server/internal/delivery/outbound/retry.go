package outbound

import (
	"context"
	"net/http"
	"sync/atomic"
	"time"
)

// JsonTransport is the bounded request boundary shared by delivery providers.
type JsonTransport interface {
	PostJson(context.Context, string, http.Header, any, time.Duration) (Response, error)
}

var retryCounter atomic.Uint64

// RetryDelay honors bounded numeric Retry-After, otherwise applying full jitter to capped exponential backoff.
func RetryDelay(attempt int, retryAfter *uint64) time.Duration {
	if retryAfter != nil {
		return time.Duration(min(*retryAfter, 60)) * time.Second
	}
	capMillis := uint64(1000 << min(max(attempt, 0), 3))
	sample := uint64(time.Now().Nanosecond()) ^ (retryCounter.Add(1) - 1)
	return time.Duration(sample%(capMillis+1)) * time.Millisecond
}

// Wait sleeps for a retry delay while allowing the caller's lifetime to end.
func Wait(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}
