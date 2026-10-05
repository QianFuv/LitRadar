package scholarly

import (
	"math"
	"math/big"
	"math/bits"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

// scheduleTime retains the persisted duration range independently of Go timer limits.
type scheduleTime struct {
	Seconds     uint64
	Nanoseconds uint32
}

var maximumScheduleTime = scheduleTime{math.MaxUint64, 999_999_999}

func milliseconds(value uint64) scheduleTime {
	return scheduleTime{value / 1000, uint32(value%1000) * 1_000_000}
}
func seconds(value uint64) scheduleTime { return scheduleTime{Seconds: value} }
func (value scheduleTime) compare(other scheduleTime) int {
	if value.Seconds < other.Seconds {
		return -1
	}
	if value.Seconds > other.Seconds {
		return 1
	}
	if value.Nanoseconds < other.Nanoseconds {
		return -1
	}
	if value.Nanoseconds > other.Nanoseconds {
		return 1
	}
	return 0
}
func later(first, second scheduleTime) scheduleTime {
	if first.compare(second) < 0 {
		return second
	}
	return first
}
func (value scheduleTime) add(other scheduleTime) scheduleTime {
	nanos := uint64(value.Nanoseconds) + uint64(other.Nanoseconds)
	whole, carry := bits.Add64(value.Seconds, other.Seconds, 0)
	if carry != 0 {
		return maximumScheduleTime
	}
	whole, carry = bits.Add64(whole, nanos/1_000_000_000, 0)
	if carry != 0 {
		return maximumScheduleTime
	}
	return scheduleTime{whole, uint32(nanos % 1_000_000_000)}
}
func (value scheduleTime) multiply(factor uint32) scheduleTime {
	nanos := uint64(value.Nanoseconds) * uint64(factor)
	high, whole := bits.Mul64(value.Seconds, uint64(factor))
	if high != 0 {
		return maximumScheduleTime
	}
	whole, carry := bits.Add64(whole, nanos/1_000_000_000, 0)
	if carry != 0 {
		return maximumScheduleTime
	}
	return scheduleTime{whole, uint32(nanos % 1_000_000_000)}
}
func (value scheduleTime) subtract(other scheduleTime) scheduleTime {
	if value.compare(other) <= 0 {
		return scheduleTime{}
	}
	whole := value.Seconds - other.Seconds
	nanos := int64(value.Nanoseconds) - int64(other.Nanoseconds)
	if nanos < 0 {
		whole--
		nanos += 1_000_000_000
	}
	return scheduleTime{whole, uint32(nanos)}
}
func (value scheduleTime) timer() time.Duration {
	return transport.Delay{Seconds: value.Seconds, Nanoseconds: value.Nanoseconds}.Duration()
}
func (value scheduleTime) millis() *big.Int {
	result := new(big.Int).SetUint64(value.Seconds)
	result.Mul(result, big.NewInt(1000))
	return result.Add(result, new(big.Int).SetUint64(uint64(value.Nanoseconds/1_000_000)))
}
func saturatedUint64(value *big.Int) uint64 {
	if value.BitLen() > 64 {
		return math.MaxUint64
	}
	return value.Uint64()
}
func phaseAtOrAfter(next, now, period scheduleTime) scheduleTime {
	if next.compare(now) >= 0 {
		return next
	}
	periodMillis := period.millis()
	if periodMillis.Sign() == 0 {
		periodMillis.SetUint64(1)
	}
	nextMillis := next.millis()
	overdue := new(big.Int).Sub(now.millis(), nextMillis)
	quotient, remainder := new(big.Int), new(big.Int)
	quotient.QuoRem(overdue, periodMillis, remainder)
	if remainder.Sign() > 0 {
		quotient.Add(quotient, big.NewInt(1))
	}
	aligned := new(big.Int).Mul(quotient, periodMillis)
	aligned.Add(aligned, nextMillis)
	return milliseconds(saturatedUint64(aligned))
}
func processContext(processId, processCount uint64) (uint32, uint32) {
	processCount = max(processCount, 1)
	processId = min(processId, processCount-1)
	return uint32(min(processId, math.MaxUint32)), uint32(min(processCount, math.MaxUint32))
}
func saturatingMultiply(first, second uint64) uint64 {
	high, low := bits.Mul64(first, second)
	if high != 0 {
		return math.MaxUint64
	}
	return low
}
func saturatingIncrement(value uint64) uint64 {
	if value == math.MaxUint64 {
		return value
	}
	return value + 1
}
