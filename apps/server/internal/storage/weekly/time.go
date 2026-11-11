// Package weekly reads immutable publication membership for fixed seven-day windows.
package weekly

import (
	"cmp"
	"time"
)

// Timestamp retains a UTC instant at nanosecond precision.
type Timestamp struct {
	Seconds     int64
	Nanoseconds uint32
}

var minimumSeconds = time.Date(0, 1, 1, 0, 0, 0, 0, time.UTC).Unix()

// FromTime preserves fractional wall-clock time for the summary's fixed window.
func FromTime(value time.Time) Timestamp { return Timestamp{value.Unix(), uint32(value.Nanosecond())} }

// Compare orders exact UTC instants.
func (stamp Timestamp) Compare(other Timestamp) int {
	if result := cmp.Compare(stamp.Seconds, other.Seconds); result != 0 {
		return result
	}
	return cmp.Compare(stamp.Nanoseconds, other.Nanoseconds)
}

// WindowStart subtracts seven days and retains the original lower-bound saturation.
func (stamp Timestamp) WindowStart() Timestamp {
	if stamp.Nanoseconds >= 1000000000 {
		stamp.Seconds++
		stamp.Nanoseconds -= 1000000000
	}
	if stamp.Seconds < minimumSeconds+7*86400 {
		return Timestamp{Seconds: minimumSeconds}
	}
	return Timestamp{stamp.Seconds - 7*86400, stamp.Nanoseconds}
}

// ParseTimestamp parses manifest timestamps using time.RFC3339Nano.
func ParseTimestamp(value string) (Timestamp, bool) {
	parsed, err := time.Parse(time.RFC3339Nano, value)
	if err != nil {
		return Timestamp{}, false
	}
	return FromTime(parsed), true
}

// Format emits UTC timestamps using RFC3339Nano or whole-second RFC3339.
func (stamp Timestamp) Format(isPrecise bool) string {
	layout := time.RFC3339
	if isPrecise {
		layout = time.RFC3339Nano
	}
	return time.Unix(stamp.Seconds, int64(stamp.Nanoseconds)).UTC().Format(layout)
}
