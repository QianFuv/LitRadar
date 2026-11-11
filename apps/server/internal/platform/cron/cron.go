// Package cron adapts five-field robfig schedules to LitRadar's UTC slot identities.
package cron

import (
	"fmt"
	"math"
	"strings"
	"time"
	_ "time/tzdata"

	calendar "github.com/robfig/cron/v3"
)

// Schedule is an immutable parsed five-field calendar expression.
type Schedule struct{ calendar calendar.Schedule }

var parser = calendar.NewParser(calendar.Minute | calendar.Hour | calendar.Dom | calendar.Month | calendar.Dow)

// Parse validates the standard five-field grammar without accepting descriptors or inline timezones.
func Parse(expression string) (Schedule, error) {
	if len(strings.Fields(expression)) != 5 {
		return Schedule{}, fmt.Errorf("cron expression must contain exactly five fields")
	}
	parsed, err := parser.Parse(expression)
	if err != nil {
		return Schedule{}, err
	}
	return Schedule{calendar: parsed}, nil
}

// Slots returns UTC identities in (checkedFrom, checkedTo], including DST folds.
func (schedule Schedule) Slots(timezone string, checkedFrom, checkedTo float64) ([]int64, error) {
	result := []int64{}
	if math.IsNaN(checkedFrom) || math.IsNaN(checkedTo) || math.IsInf(checkedFrom, 0) || math.IsInf(checkedTo, 0) {
		return nil, fmt.Errorf("invalid schedule time interval")
	}
	if checkedFrom >= checkedTo {
		return result, nil
	}
	location, err := Location(timezone)
	if err != nil {
		return nil, fmt.Errorf("timezone must be a valid IANA name")
	}
	start := time.Unix(int64(math.Floor(checkedFrom/60))*60, 0).In(location)
	for next := schedule.calendar.Next(start); !next.IsZero() && float64(next.Unix()) <= checkedTo; next = schedule.calendar.Next(next) {
		result = append(result, next.Unix())
	}
	return result, nil
}
