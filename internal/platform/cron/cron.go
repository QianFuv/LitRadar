// Package cron preserves LitRadar's five-field calendar matching and UTC slot identity.
package cron

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"
	_ "time/tzdata"
)

// Schedule is an immutable parsed expression; day wildcard syntax remains significant.
type Schedule struct {
	fields      [5]uint64
	dayWildcard bool
}

// Parse validates five fields without changing single-value steps or Sunday range semantics.
func Parse(expression string) (Schedule, error) {
	parts := strings.Fields(expression)
	if len(parts) != 5 {
		return Schedule{}, fmt.Errorf("Cron expression must contain exactly five fields")
	}
	schedule := Schedule{dayWildcard: strings.HasPrefix(parts[2], "*") || strings.HasPrefix(parts[4], "*")}
	minimums := [5]int64{0, 0, 1, 1, 0}
	maximums := [5]int64{59, 23, 31, 12, 7}
	for index, field := range parts {
		var names []string
		if index == 3 {
			names = []string{"jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"}
		}
		if index == 4 {
			names = []string{"sun", "mon", "tue", "wed", "thu", "fri", "sat"}
		}
		for _, part := range strings.Split(field, ",") {
			values, err := parsePart(strings.TrimSpace(part), minimums[index], maximums[index], names)
			if err != nil {
				return Schedule{}, err
			}
			schedule.fields[index] |= values
		}
	}
	if schedule.fields[4]&(1<<7) != 0 {
		schedule.fields[4] |= 1
	}
	return schedule, nil
}

func parseValue(value string, minimum, maximum int64, names []string) (int64, error) {
	normalized := strings.ToLower(strings.TrimSpace(value))
	for index, name := range names {
		if normalized == name {
			return int64(index) + minimum, nil
		}
	}
	parsed, err := strconv.ParseInt(normalized, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("cron field contains an invalid value")
	}
	if parsed < minimum || parsed > maximum {
		return 0, fmt.Errorf("cron field value is outside the allowed range")
	}
	return parsed, nil
}

func parsePart(part string, minimum, maximum int64, names []string) (uint64, error) {
	if part == "" {
		return 0, fmt.Errorf("empty cron field part")
	}
	base, stepText, hasStep := strings.Cut(part, "/")
	step := int64(1)
	if hasStep {
		parsed, err := strconv.ParseInt(stepText, 10, 64)
		if err != nil || parsed <= 0 {
			return 0, fmt.Errorf("cron step must be a positive integer")
		}
		step = parsed
	}
	start, end := minimum, maximum
	if base != "*" {
		startText, endText, hasRange := strings.Cut(base, "-")
		var err error
		start, err = parseValue(startText, minimum, maximum, names)
		if err != nil {
			return 0, err
		}
		end = start
		if hasRange {
			end, err = parseValue(endText, minimum, maximum, names)
			if err != nil {
				return 0, err
			}
		}
		if start > end {
			return 0, fmt.Errorf("cron range start must be less than or equal to end")
		}
	}
	var values uint64
	for candidate := start; candidate <= end; candidate++ {
		if (candidate-start)%step == 0 {
			values |= 1 << uint(candidate)
		}
	}
	return values, nil
}

// Matches applies calendar matching in the timestamp's location, including restricted-day OR.
func (schedule Schedule) Matches(local time.Time) bool {
	values := [5]int{local.Minute(), local.Hour(), local.Day(), int(local.Month()), int(local.Weekday())}
	var matches [5]bool
	for index, value := range values {
		matches[index] = schedule.fields[index]&(1<<uint(value)) != 0
	}
	day := matches[2] || matches[4]
	if schedule.dayWildcard {
		day = matches[2] && matches[4]
	}
	return matches[0] && matches[1] && day && matches[3]
}

// Slots returns minute-aligned UTC identities in (checkedFrom, checkedTo], including DST folds.
// Embedded Go timezone data remains available in minimal containers without system zoneinfo.
func (schedule Schedule) Slots(timezone string, checkedFrom, checkedTo float64) ([]int64, error) {
	result := []int64{}
	if checkedFrom >= checkedTo {
		return result, nil
	}
	location, err := Location(timezone)
	if err != nil {
		return nil, fmt.Errorf("timezone must be a valid IANA name")
	}
	if math.IsNaN(checkedFrom) || math.IsNaN(checkedTo) || math.IsInf(checkedFrom, 0) || math.IsInf(checkedTo, 0) {
		return nil, fmt.Errorf("invalid schedule time interval")
	}
	first := int64(math.Floor(checkedFrom/60)) + 1
	last := int64(math.Floor(checkedTo / 60))
	for minute := first; minute <= last; minute++ {
		instant := time.Unix(minute*60, 0)
		if schedule.Matches(instant.In(location)) {
			result = append(result, instant.Unix())
		}
	}
	return result, nil
}
