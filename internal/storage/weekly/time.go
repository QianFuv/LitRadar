// Package weekly reads immutable publication membership for fixed seven-day windows.
package weekly

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

// Timestamp retains Chrono's leap-second representation rather than normalizing it into the following minute.
type Timestamp struct {
	Seconds     int64
	Nanoseconds uint32
}

var minimumSeconds = time.Date(-262143, 1, 1, 0, 0, 0, 0, time.UTC).Unix()
var maximumSeconds = time.Date(262142, 12, 31, 23, 59, 59, 0, time.UTC).Unix()

// FromTime preserves fractional wall-clock time for the summary's fixed window.
func FromTime(value time.Time) Timestamp { return Timestamp{value.Unix(), uint32(value.Nanosecond())} }

// Compare orders exact UTC instants using the original seconds/nanoseconds tuple.
func (stamp Timestamp) Compare(other Timestamp) int {
	if stamp.Seconds < other.Seconds {
		return -1
	}
	if stamp.Seconds > other.Seconds {
		return 1
	}
	if stamp.Nanoseconds < other.Nanoseconds {
		return -1
	}
	if stamp.Nanoseconds > other.Nanoseconds {
		return 1
	}
	return 0
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

func fixedDigits(value string) (int, bool) {
	result := 0
	for index := range len(value) {
		if value[index] < '0' || value[index] > '9' {
			return 0, false
		}
		result = result*10 + int(value[index]-'0')
	}
	return result, true
}

// ParseTimestamp implements the frozen RFC3339 grammar, including lowercase separators and leap seconds.
func ParseTimestamp(value string) (Timestamp, bool) {
	value = strings.TrimSpace(value)
	if len(value) < 20 || value[4] != '-' || value[7] != '-' || (value[10] != 'T' && value[10] != 't' && value[10] != ' ') || value[13] != ':' || value[16] != ':' {
		return Timestamp{}, false
	}
	parts := []string{value[:4], value[5:7], value[8:10], value[11:13], value[14:16], value[17:19]}
	numbers := make([]int, len(parts))
	for index, part := range parts {
		number, ok := fixedDigits(part)
		if !ok {
			return Timestamp{}, false
		}
		numbers[index] = number
	}
	year, month, day, hour, minute, second := numbers[0], numbers[1], numbers[2], numbers[3], numbers[4], numbers[5]
	if hour > 23 || minute > 59 || second > 60 {
		return Timestamp{}, false
	}
	leap := uint32(0)
	if second == 60 {
		second = 59
		leap = 1000000000
	}
	date := time.Date(year, time.Month(month), day, hour, minute, second, 0, time.UTC)
	if date.Year() != year || int(date.Month()) != month || date.Day() != day {
		return Timestamp{}, false
	}
	position := 19
	fraction := uint32(0)
	if value[position] == '.' {
		position++
		start := position
		for position < len(value) && value[position] >= '0' && value[position] <= '9' {
			if position-start < 9 {
				fraction = fraction*10 + uint32(value[position]-'0')
			}
			position++
		}
		if position == start {
			return Timestamp{}, false
		}
		for digits := position - start; digits < 9; digits++ {
			fraction *= 10
		}
	}
	zone := value[position:]
	offset := 0
	if zone != "Z" && zone != "z" {
		negative := false
		if strings.HasPrefix(zone, "−") {
			negative = true
			zone = zone[len("−"):]
		} else if strings.HasPrefix(zone, "-") {
			negative = true
			zone = zone[1:]
		} else if strings.HasPrefix(zone, "+") {
			zone = zone[1:]
		} else {
			return Timestamp{}, false
		}
		if len(zone) != 5 || zone[2] != ':' {
			return Timestamp{}, false
		}
		hours, ok := fixedDigits(zone[:2])
		if !ok || hours > 23 {
			return Timestamp{}, false
		}
		minutes, ok := fixedDigits(zone[3:])
		if !ok || minutes > 59 {
			return Timestamp{}, false
		}
		offset = hours*3600 + minutes*60
		if negative {
			offset = -offset
		}
	}
	return Timestamp{date.Unix() - int64(offset), fraction + leap}, true
}

func parseManifestTime(value string) (Timestamp, bool) {
	if stamp, ok := ParseTimestamp(value); ok {
		return stamp, true
	}
	value = strings.TrimSpace(value)
	seconds, err := strconv.ParseInt(value, 10, 64)
	if err != nil || strconv.FormatInt(seconds, 10) != value || seconds < minimumSeconds || seconds > maximumSeconds {
		return Timestamp{}, false
	}
	return Timestamp{Seconds: seconds}, true
}

// Format uses the original RFC3339 whole-second or 0/3/6/9-digit fractional precision.
func (stamp Timestamp) Format(isPrecise bool) string {
	date := time.Unix(stamp.Seconds, 0).UTC()
	year := fmt.Sprintf("%04d", date.Year())
	if date.Year() < 0 {
		year = fmt.Sprintf("-%04d", -date.Year())
	} else if date.Year() > 9999 {
		year = "+" + strconv.Itoa(date.Year())
	}
	second := date.Second()
	fraction := stamp.Nanoseconds
	if fraction >= 1000000000 {
		second++
		fraction -= 1000000000
	}
	result := fmt.Sprintf("%s-%02d-%02dT%02d:%02d:%02d", year, date.Month(), date.Day(), date.Hour(), date.Minute(), second)
	if isPrecise && fraction != 0 {
		if fraction%1000000 == 0 {
			result += fmt.Sprintf(".%03d", fraction/1000000)
		} else if fraction%1000 == 0 {
			result += fmt.Sprintf(".%06d", fraction/1000)
		} else {
			result += fmt.Sprintf(".%09d", fraction)
		}
	}
	return result + "Z"
}
