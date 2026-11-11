package transport

import (
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// Delay retains the full unsigned server delay without time.Duration overflow.
type Delay struct {
	Seconds     uint64
	Nanoseconds uint32
}

// Duration saturates delays beyond Go's timer range; those cannot fit a request budget.
func (delay Delay) Duration() time.Duration {
	if delay.Seconds > uint64(math.MaxInt64)/1_000_000_000 || delay.Seconds == uint64(math.MaxInt64)/1_000_000_000 && uint64(delay.Nanoseconds) > uint64(math.MaxInt64)%1_000_000_000 {
		return time.Duration(math.MaxInt64)
	}
	return time.Duration(delay.Seconds)*time.Second + time.Duration(delay.Nanoseconds)
}

// RetryAfter reads the first visible-ASCII Retry-After header.
func RetryAfter(headers http.Header) (Delay, bool) {
	value := headers.Get("Retry-After")
	for _, character := range []byte(value) {
		if character != '\t' && (character < 0x20 || character > 0x7e) {
			return Delay{}, false
		}
	}
	return ParseRetryAfter(value, time.Now())
}

// ParseRetryAfter accepts unsigned seconds or an HTTP date parsed by net/http.
func ParseRetryAfter(value string, now time.Time) (Delay, bool) {
	value = strings.TrimSpace(value)
	if seconds, err := strconv.ParseUint(value, 10, 64); err == nil {
		return Delay{Seconds: seconds}, true
	}
	date, err := http.ParseTime(value)
	if err != nil {
		return Delay{}, false
	}
	if !date.After(now) {
		return Delay{}, true
	}
	seconds := date.Unix() - now.Unix()
	nanoseconds := int64(date.Nanosecond()) - int64(now.Nanosecond())
	if nanoseconds < 0 {
		seconds--
		nanoseconds += int64(time.Second)
	}
	return Delay{Seconds: uint64(seconds), Nanoseconds: uint32(nanoseconds)}, true
}
